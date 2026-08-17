//! `sirji` — the command line.
//!
//! Does no networking of its own: everything that touches the network is asked of
//! the daemon over the unix socket in `$SIRJI_HOME`. Filesystem permission on that
//! socket is the authorization, which is why this channel has no keys and no
//! policy on it.
//!
//! Hand-rolled argument matching for now; `clap` arrives when there is enough
//! surface to justify it.

use anyhow::{Result, bail};
use sirji::daemon;
use sirji::proto::{Invite, Request, Response};
use sirji::{Keystore, id52};

const USAGE: &str = "\
sirji — peer-to-peer network substrate

  sirji init                    create $SIRJI_HOME with its first handshake key
  sirji daemon                  run the daemon for this $SIRJI_HOME (foreground)
  sirji status                  what the daemon is listening as
  sirji address new <alias>     mint another handshake key
  sirji invite <alias>          mint an identity for someone; print an invite
  sirji accept <alias> <invite> complete an invite and pair
  sirji peers                   every relationship, pending or established
  sirji device invite <name>    mint an enrolment invite for a device
  sirji devices                 our own fleet, and which are connected
  sirji doctor                  why isn't this working? checks home, keys, DNS,
                                UDP egress and every relay, and says what each
                                failure means. Needs no daemon.
  sirji net check               validate network.toml without a daemon
  sirji key ls                  list the keystore, verifying every entry

An instance is its $SIRJI_HOME (default ~/.sirji), so two sirjis on one machine
are two directories and nothing else. Every command but `init` and `daemon` talks
to the running daemon over the socket in that directory.";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();

    match args.as_slice() {
        [] | ["-h"] | ["--help"] | ["help"] => {
            println!("{USAGE}");
            Ok(())
        }
        ["init"] => init(),
        ["daemon"] => daemon_run(),
        ["key", "ls"] => key_ls(),
        ["doctor"] => doctor(),
        ["net", "check"] => net_check(),
        ["status"] => ask(Request::Status),
        ["peers"] => ask(Request::Peers),
        ["devices"] => ask(Request::Devices),
        ["device", "invite", name] => ask(Request::DeviceInvite {
            name: (*name).to_string(),
        }),
        ["address", "new", alias] => ask(Request::NewAddress {
            alias: (*alias).to_string(),
        }),
        ["invite", alias] => ask(Request::Invite {
            alias: (*alias).to_string(),
        }),
        ["accept", alias, invite] => ask(Request::Accept {
            alias: (*alias).to_string(),
            invite: Invite::decode(invite)?,
        }),
        _ => {
            eprintln!("{USAGE}");
            bail!("unrecognised command: {}", args.join(" "));
        }
    }
}

fn init() -> Result<()> {
    let home = sirji::keystore::home()?;
    let (home, key) = daemon::init(&home)?;
    println!("sirji home {}", home.display());
    println!("handshake key `default` {key}");
    println!("\nstart it with `sirji daemon`.");
    Ok(())
}

fn daemon_run() -> Result<()> {
    let home = sirji::keystore::home()?;
    if !sirji::Network::path_in(&home).exists() {
        bail!("no sirji at {} — run `sirji init` first", home.display());
    }
    println!("sirji home {}", home.display());
    tokio::runtime::Runtime::new()?.block_on(async {
        let daemon = sirji::Daemon::start(home).await?;
        daemon.run().await
    })
}

fn key_ls() -> Result<()> {
    let store = Keystore::open()?;
    let keys = store.list()?;
    if keys.is_empty() {
        eprintln!("no keys in {}", store.dir().display());
        return Ok(());
    }
    for key in &keys {
        // Loading verifies the secret really matches the name it is filed under.
        store.secret(key)?;
        println!("{}", id52::encode(key));
    }
    eprintln!("{} key(s), all verified", keys.len());
    Ok(())
}

/// The one command that does its own networking.
///
/// Everything else asks the daemon, but the daemon not running is one of the things
/// this has to be able to diagnose — so it talks to the network directly, and says
/// so rather than quietly making an exception.
fn doctor() -> Result<()> {
    let home = sirji::keystore::home()?;
    let healthy = tokio::runtime::Runtime::new()?.block_on(sirji::doctor::run(&home))?;
    if !healthy {
        std::process::exit(1);
    }
    Ok(())
}

fn net_check() -> Result<()> {
    let home = sirji::keystore::home()?;
    let net = sirji::Network::load(&home)?;
    net.check()?;

    println!("{} — ok", sirji::Network::path_in(&home).display());
    for hk in &net.handshake_keys {
        let used_by = net
            .peers
            .iter()
            .filter(|p| p.reached_on.as_deref() == Some(hk.alias.as_str()))
            .count();
        let state = if hk.retired { "retired" } else { "current" };
        println!(
            "  address {:<12} {state:<8} {used_by} peer(s) reached here",
            hk.alias
        );
    }
    for key in net.drained() {
        println!("  address {} is drained and can be unbound", key.alias);
    }
    let pending = net.peers.iter().filter(|p| p.is_pending()).count();
    println!("  {} peer(s), {pending} pending", net.peers.len());
    println!("  {} device(s)", net.devices.len());
    Ok(())
}

fn ask(request: Request) -> Result<()> {
    let home = sirji::keystore::home()?;
    let response = tokio::runtime::Runtime::new()?.block_on(daemon::ask(&home, &request))?;
    render(response)
}

fn render(response: Response) -> Result<()> {
    match response {
        Response::Status {
            home,
            addresses,
            peers,
            pending,
        } => {
            println!("home     {home}");
            for a in addresses {
                let state = if a.retired { "retired" } else { "current" };
                let bound = if a.bound { "bound" } else { "NOT BOUND" };
                println!("address  {:<10} {state:<8} {bound:<10} {}", a.alias, a.key);
                for r in &a.relays {
                    let state = if r.connected { "connected" } else { "DOWN" };
                    match &r.error {
                        Some(e) => println!("  relay  {state:<10} {} — {e}", r.url),
                        None => println!("  relay  {state:<10} {}", r.url),
                    }
                }
            }
            println!("peers    {peers} established, {pending} pending");
        }
        Response::Invite { invite } => {
            println!("{}", invite.encode());
            eprintln!(
                "\nsend that to them. they run:\n\n    sirji accept <their-name-for-you> <invite>\n"
            );
        }
        Response::Accepted { alias } => println!("paired with {alias}"),
        Response::Peers { peers } => {
            if peers.is_empty() {
                println!("no peers yet — `sirji invite <alias>` to start one");
            }
            for p in peers {
                match p.peer {
                    None => println!("{:<12} pending invite", p.alias),
                    Some(their) => {
                        println!("{:<12} {}", p.alias, their);
                        println!("{:<12}   we are {} to them", "", p.mine);
                        for a in &p.addresses {
                            println!("{:<12}   reach at {a}", "");
                        }
                        if let Some(on) = &p.reached_on {
                            println!("{:<12}   last arrived on our `{on}`", "");
                        }
                    }
                }
            }
        }
        Response::Devices { devices } => {
            if devices.is_empty() {
                println!("no devices — `sirji device invite <name>` to enrol one");
            }
            for d in devices {
                if d.pending {
                    println!("{:<12} awaiting enrolment", d.name);
                    continue;
                }
                let live = if d.live { "live" } else { "not connected" };
                println!("{:<12} {live:<14} {}", d.name, d.keys.join(", "));
            }
        }
        Response::NewAddress { alias, key } => {
            println!("handshake key `{alias}` {key}");
            eprintln!("restart the daemon to bind it.");
        }
        Response::Error { message } => bail!("{message}"),
    }
    Ok(())
}
