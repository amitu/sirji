//! `sirji doctor` — why isn't this working?
//!
//! Written after spending a session working that out by hand on a corporate
//! network, where the answer turned out to be specific and non-obvious: **UDP
//! egress was fine and every relay hostname returned a filter's block page**. Those
//! are opposite conclusions — one says the network is hopeless, the other says move
//! the coordination somewhere the filter has not categorised — and nothing in a
//! connection timeout distinguishes them.
//!
//! So each check here answers a question that leads somewhere different:
//!
//! | check | if it fails |
//! |---|---|
//! | home, network, keys | local; nothing to do with the network |
//! | daemon | start it |
//! | dns | the network has no working resolver, or blocks these names |
//! | udp egress | QUIC cannot work here at all — the one fatal verdict |
//! | relays | coordination is blocked; run your own, or set `SIRJI_RELAY` |
//!
//! It deliberately does **not** need a running daemon. The moment you most want a
//! diagnosis is when nothing is up.

use std::time::{Duration, Instant};

use anyhow::Result;

use crate::{Network, Settings, id52};

/// A public STUN server, used only to prove packets get out and back.
///
/// STUN because it is the closest thing to what QUIC actually needs: an outbound
/// UDP datagram to an arbitrary port, and a reply arriving on the same socket. A
/// TCP or HTTP probe would pass on networks where QUIC still cannot work.
const STUN_SERVER: &str = "stun.l.google.com:19302";

/// Names worth resolving. Failure here is usually a proxy-only network.
const DNS_PROBES: &[&str] = &["dns.iroh.link", "stun.l.google.com"];

const PATIENCE: Duration = Duration::from_secs(5);
/// Relays connect through TLS and HTTP, so they get longer than a UDP round trip.
const RELAY_PATIENCE: Duration = Duration::from_secs(12);

/// What one check concluded.
enum Verdict {
    Ok(String),
    /// Worth knowing, but not broken.
    Note(String),
    Bad { detail: String, fix: String },
}

fn say(step: &str, verdict: &Verdict) {
    match verdict {
        Verdict::Ok(detail) => println!("  ok    {step:<14} {detail}"),
        Verdict::Note(detail) => println!("  --    {step:<14} {detail}"),
        Verdict::Bad { detail, fix } => {
            println!("  FAIL  {step:<14} {detail}");
            println!("        {:<14} → {fix}", "");
        }
    }
}

/// Run every check. Returns false if anything is actually broken.
pub async fn run(home: &std::path::Path) -> Result<bool> {
    println!("sirji doctor — {}", home.display());
    let settings = Settings::load(home)?;
    let mut broken = 0;

    for (step, verdict) in [
        ("home", check_home(home)),
        ("keys", check_keys(home)),
        ("daemon", check_daemon(home)),
    ] {
        say(step, &verdict);
        broken += usize::from(matches!(verdict, Verdict::Bad { .. }));
    }

    let dns = check_dns().await;
    say("dns", &dns);
    broken += usize::from(matches!(dns, Verdict::Bad { .. }));

    let udp = check_udp().await;
    say("udp egress", &udp);
    let udp_broken = matches!(udp, Verdict::Bad { .. });
    broken += usize::from(udp_broken);

    for verdict in check_relays(&settings).await {
        say("relay", &verdict);
        broken += usize::from(matches!(verdict, Verdict::Bad { .. }));
    }

    println!();
    if udp_broken {
        // Said separately because it is the one verdict that is not a
        // configuration problem: without outbound UDP there is nothing to fix here.
        println!("UDP cannot get out, so QUIC cannot work on this network at all.");
        println!("Everything else is beside the point until that changes.");
    } else if broken > 0 {
        let count = if broken == 1 { "1 problem".to_string() } else { format!("{broken} problems") };
        println!("{count} above. UDP works, so this is fixable:");
        println!("a relay you run — on a domain your network already trusts — is");
        println!("usually the whole answer. Set SIRJI_RELAY to it.");
    } else {
        println!("nothing wrong here.");
    }
    Ok(broken == 0)
}

fn check_home(home: &std::path::Path) -> Verdict {
    if !home.exists() {
        return Verdict::Bad {
            detail: format!("{} does not exist", home.display()),
            fix: "run `sirji init`".into(),
        };
    }
    match Network::load(home) {
        Err(e) => Verdict::Bad {
            detail: format!("{e:#}"),
            fix: "run `sirji init`, or fix network.toml by hand".into(),
        },
        Ok(net) => match net.check() {
            Err(e) => Verdict::Bad {
                detail: format!("network.toml is not usable: {e:#}"),
                fix: "see `sirji net check`".into(),
            },
            Ok(()) => Verdict::Ok(format!(
                "{} address(es), {} peer(s), {} device(s)",
                net.handshake_keys.len(),
                net.peers.len(),
                net.devices.len()
            )),
        },
    }
}

/// Every key `network.toml` says we listen on must be one we can actually use.
///
/// A missing private key is silent otherwise: the address is advertised, peers dial
/// it, and nothing answers.
fn check_keys(home: &std::path::Path) -> Verdict {
    let Ok(net) = Network::load(home) else {
        return Verdict::Note("skipped: no readable network.toml".into());
    };
    let store = crate::Keystore::at(home.join("keys"));

    let mut missing = Vec::new();
    for hk in &net.handshake_keys {
        let usable = id52::decode(&hk.key)
            .ok()
            .and_then(|key| store.secret(&key).ok())
            .is_some();
        if !usable {
            missing.push(hk.alias.clone());
        }
    }

    if missing.is_empty() {
        Verdict::Ok(format!("{} private key(s), all readable", net.handshake_keys.len()))
    } else {
        Verdict::Bad {
            detail: format!("no usable private key for {missing:?}"),
            fix: "restore the keys/ directory, or `sirji address new` and re-share".into(),
        }
    }
}

/// Is a daemon up? Not running is a **note**, not a failure — doctor exists
/// precisely for when nothing is running, and reporting that as broken would put a
/// red line in front of everyone diagnosing a fresh install.
///
/// A leftover socket from a killed daemon is a note for the same reason: the daemon
/// unlinks it before binding, so there is nothing for anybody to clean up. Saying
/// "remove the stale socket" would be inventing a chore.
fn check_daemon(home: &std::path::Path) -> Verdict {
    let socket = home.join(crate::daemon::SOCKET);
    if !socket.exists() {
        return Verdict::Note("not running (`sirji daemon` starts it)".into());
    }
    match std::os::unix::net::UnixStream::connect(&socket) {
        Ok(_) => Verdict::Ok("running".into()),
        Err(_) => Verdict::Note(
            "not running — a socket is left over from last time, which `sirji daemon` clears"
                .into(),
        ),
    }
}

async fn check_dns() -> Verdict {
    let mut failed = Vec::new();
    for name in DNS_PROBES {
        let lookup = tokio::time::timeout(
            PATIENCE,
            tokio::net::lookup_host(format!("{name}:443")),
        )
        .await;
        let resolved = match lookup {
            Ok(Ok(mut addrs)) => addrs.next().is_some(),
            _ => false,
        };
        if !resolved {
            failed.push(*name);
        }
    }

    if failed.is_empty() {
        Verdict::Ok(format!("resolved {}", DNS_PROBES.join(", ")))
    } else {
        Verdict::Bad {
            detail: format!("cannot resolve {failed:?}"),
            fix: "a network that resolves nothing usually wants a proxy; \
                  sirji needs real DNS and real UDP"
                .into(),
        }
    }
}

/// Send a STUN binding request and see whether an answer comes back.
///
/// Hand-built because it is twenty bytes and pulling in a STUN crate to write them
/// would be a dependency to keep current forever. The reply only has to *be* a
/// reply — what it says about our address is not this check's business.
async fn check_udp() -> Verdict {
    async fn probe() -> Result<Duration> {
        let socket = tokio::net::UdpSocket::bind("0.0.0.0:0").await?;
        let server = tokio::net::lookup_host(STUN_SERVER)
            .await?
            .next()
            .ok_or_else(|| anyhow::anyhow!("{STUN_SERVER} does not resolve"))?;

        let mut request = Vec::with_capacity(20);
        request.extend_from_slice(&0x0001u16.to_be_bytes()); // binding request
        request.extend_from_slice(&0u16.to_be_bytes()); // no attributes
        request.extend_from_slice(&0x2112_A442u32.to_be_bytes()); // magic cookie
        request.extend_from_slice(&[7u8; 12]); // transaction id

        let started = Instant::now();
        socket.send_to(&request, server).await?;

        let mut buf = [0u8; 512];
        let (read, _) = tokio::time::timeout(PATIENCE, socket.recv_from(&mut buf)).await??;
        if read < 20 || u16::from_be_bytes([buf[0], buf[1]]) != 0x0101 {
            anyhow::bail!("something answered, but not with a STUN response");
        }
        Ok(started.elapsed())
    }

    match probe().await {
        Ok(took) => Verdict::Ok(format!("{STUN_SERVER} answered in {}ms", took.as_millis())),
        Err(e) => Verdict::Bad {
            detail: format!("no answer from {STUN_SERVER}: {e:#}"),
            fix: "QUIC needs outbound UDP. If this is a corporate network, this is \
                  the thing to ask for — not a proxy exception"
                .into(),
        },
    }
}

/// Bind a throwaway endpoint and report what iroh makes of each relay.
///
/// Asking iroh rather than probing the URLs ourselves: it is iroh that has to be
/// satisfied, and its error text already distinguishes the cases that matter — a
/// TLS failure reads differently from a filter's block page, and both read
/// differently from a name that does not resolve.
async fn check_relays(settings: &Settings) -> Vec<Verdict> {
    use iroh::Watcher;

    if settings.relays_configured() && settings.relay.is_empty() {
        return vec![Verdict::Note(
            "none configured — direct connectivity only, by choice".into(),
        )];
    }

    // A throwaway key: this endpoint dials nothing and is closed in a moment. It
    // never touches the keystore, so a doctor run leaves no trace.
    let secret = crate::SecretKey::generate();
    let endpoint = match crate::bind_dialer(secret).await {
        Ok(endpoint) => endpoint,
        Err(e) => {
            return vec![Verdict::Bad {
                detail: format!("cannot bind an endpoint at all: {e:#}"),
                fix: "this is local — check whether anything can open a UDP socket".into(),
            }];
        }
    };

    // Give iroh time to try. A relay that has not answered yet and one that never
    // will look identical for the first second or two.
    let deadline = Instant::now() + RELAY_PATIENCE;
    let mut statuses = Vec::new();
    while Instant::now() < deadline {
        statuses = endpoint.home_relay_status().get();
        if statuses.iter().any(|s| s.is_connected()) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    let mut verdicts = Vec::new();
    for status in &statuses {
        let url = status.url().to_string();
        if status.is_connected() {
            verdicts.push(Verdict::Ok(format!("{url} connected")));
        } else {
            // The whole chain, not just the outermost message. "Failed to connect
            // to relay server" is true of every failure and useful for none of
            // them; the cause underneath is what picks the advice, so it had
            // better be the thing on screen too.
            let why = match status.last_error() {
                Some(e) => error_chain(e),
                None => "no answer".to_string(),
            };
            verdicts.push(Verdict::Bad {
                detail: format!("{url} — {why}"),
                fix: relay_advice(&why),
            });
        }
    }
    if verdicts.is_empty() {
        verdicts.push(Verdict::Bad {
            detail: format!("no relay connected within {RELAY_PATIENCE:?}"),
            fix: relay_advice(""),
        });
    }

    endpoint.close().await;
    verdicts
}

/// Flatten an error and everything under it into one line.
fn error_chain(error: &dyn std::error::Error) -> String {
    let mut out = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        out.push_str(&format!(": {cause}"));
        source = cause.source();
    }
    out
}

/// The advice depends on *how* it failed, because the fixes are unrelated.
fn relay_advice(error: &str) -> String {
    let lower = error.to_lowercase();
    if lower.contains("certificate") || lower.contains("unknownissuer") || lower.contains("tls") {
        "TLS was intercepted and the interceptor's CA is not trusted here. Add it \
         to the system store, or point SIRJI_EXTRA_CA at the PEM."
            .into()
    } else if lower.contains("403") || lower.contains("forbidden") || lower.contains("blocked") {
        "a filter is blocking this hostname by category, whatever the certificate \
         says. Run a relay on a domain your network already trusts and set \
         SIRJI_RELAY to it."
            .into()
    } else {
        "if UDP works but no relay does, coordination is what is blocked. Run your \
         own relay and set SIRJI_RELAY — peers still connect directly."
            .into()
    }
}
