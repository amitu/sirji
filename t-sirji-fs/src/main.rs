//! `t-sirji-fs` — a sirji device that serves a directory.
//!
//! The reference consumer of the sirji crate. It embeds sirji and *is* a device:
//! its own home, its own keypair, a parent sirji it enrols with by handshake, and
//! a name that peers of its parent can resolve.
//!
//! It reaches its parent over iroh with its own key, exactly as it would from
//! another machine. Being on the same box buys it nothing and is never detected.

mod config;
mod proto;

use anyhow::{Result, bail};
use config::Config;
use sirji::proto::{Hello, Welcome};
use sirji::{Keystore, id52};
use tokio::io::{AsyncBufReadExt, BufReader};

const USAGE: &str = "\
t-sirji-fs — a sirji device that serves a directory

  t-sirji-fs init --parent <invite> [--name <name>] [--root <dir>]
        create $TSF_HOME, mint a key, and enrol with the parent that issued
        the invite. Get one with `sirji device invite <name>`.

  t-sirji-fs serve
        register with the parent and serve the directory

  t-sirji-fs status
        what this device is and where it belongs

  t-sirji-fs ls  <name@peer> [path]
  t-sirji-fs get <name@peer> <path>
        read from another t-sirji-fs. Our parent resolves `peer`, asks them
        where `name` is, and returns a ticket bound to our key. A bare id52
        still dials, but a served device refuses a dial with no ticket.

$TSF_HOME defaults to ~/.t-sirji-fs. A device has its own home because a device
may be on another machine.";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();

    match args.as_slice() {
        [] | ["-h"] | ["--help"] | ["help"] => {
            println!("{USAGE}");
            Ok(())
        }
        ["status"] => status(),
        ["ls", target] => rt()?.block_on(client_ls(target, "")),
        ["ls", target, path] => rt()?.block_on(client_ls(target, path)),
        ["get", target, path] => rt()?.block_on(client_get(target, path)),
        ["serve"] => rt()?.block_on(serve()),
        ["init", rest @ ..] => rt()?.block_on(init(rest)),
        _ => {
            eprintln!("{USAGE}");
            bail!("unrecognised command: {}", args.join(" "));
        }
    }
}

fn rt() -> Result<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Runtime::new()?)
}

fn keystore(home: &std::path::Path) -> Keystore {
    Keystore::at(home.join("keys"))
}

fn status() -> Result<()> {
    let home = Config::home()?;
    let config = Config::load(&home)?;
    println!("home     {}", home.display());
    println!("name     {}", config.name);
    println!("key      {}", config.key);
    println!("root     {}", config.root.display());
    for address in &config.parent {
        println!("parent   {address}");
    }
    Ok(())
}

async fn init(args: &[&str]) -> Result<()> {
    let mut invite = None;
    let mut name = None;
    let mut root = None;

    let mut i = 0;
    while i < args.len() {
        match args[i] {
            "--parent" => {
                invite = args.get(i + 1).copied();
                i += 2;
            }
            "--name" => {
                name = args.get(i + 1).copied();
                i += 2;
            }
            "--root" => {
                root = args.get(i + 1).copied();
                i += 2;
            }
            other => bail!("unrecognised argument: {other}"),
        }
    }
    let invite = invite.ok_or_else(|| {
        anyhow::anyhow!("--parent <invite> is required; get one with `sirji device invite <name>`")
    })?;
    let invite = sirji::proto::Invite::decode(invite)?;

    let home = Config::home()?;
    if Config::path_in(&home).exists() {
        bail!("{} already exists", Config::path_in(&home).display());
    }
    std::fs::create_dir_all(&home)?;

    // Our own key, in our own keystore. The parent never sees the secret half and
    // could not use it if it did.
    let keys = keystore(&home);
    let key = keys.generate()?;
    let secret = keys.secret(&key)?;

    let root = root
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| home.join("served"));
    std::fs::create_dir_all(&root)?;

    let config = Config {
        // The name the parent minted the invite for is authoritative; the flag is a
        // convenience for the common case where they match.
        name: name.unwrap_or("fs").to_string(),
        key: id52::encode(&key),
        parent: invite.addresses.clone(),
        parent_dns: invite.dns.clone(),
        parent_hints: invite.hints.clone(),
        root: root.clone(),
    };

    // Enrol: dial the parent as ourselves and present the identity it minted for
    // us. Since that identity went to nobody else, presenting it proves we are the
    // invitee — no approval step, exactly as pairing.
    println!("enrolling as {}", config.key);
    let endpoint = sirji::endpoint::bind_dialer(secret).await?;
    let welcome = sirji::daemon::greet_parent(
        &endpoint,
        &invite,
        Hello::Invited {
            invited_to: invite.identity.clone(),
            addresses: vec![config.key.clone()],
            dns: Vec::new(),
        },
    )
    .await?;
    endpoint.close().await;

    let name = match welcome {
        Welcome::Ok { alias, .. } => alias,
        Welcome::No { reason } => bail!("the parent refused us: {reason}"),
    };

    let config = Config { name: name.clone(), ..config };
    config.save(&home)?;

    println!("enrolled as `{name}`");
    println!("home     {}", home.display());
    println!("serving  {}", root.display());
    println!("\nstart it with `t-sirji-fs serve`.");
    Ok(())
}

async fn serve() -> Result<()> {
    let home = Config::home()?;
    sirji::Settings::load(&home)?.activate();
    let config = Config::load(&home)?;
    let keys = keystore(&home);
    let key = id52::decode(&config.key)?;
    let secret = keys.secret(&key)?;

    // We listen, because a peer that resolved our name dials us directly. The
    // parent is a doorman, never a proxy.
    let endpoint = sirji::bind(secret).await?;
    println!("device `{}` listening as {}", config.name, config.key);
    println!("serving {}", config.root.display());

    // Registering is holding a connection open: while it is up we are live, and
    // when it drops we are not. No heartbeat, no timeout to tune.
    let listening: Vec<String> = endpoint
        .bound_sockets()
        .iter()
        .map(|a| format!("127.0.0.1:{}", a.port()))
        .collect();
    let registration = tokio::spawn(register(config.clone(), home.clone(), listening));

    let config = std::sync::Arc::new(config);
    while let Some(incoming) = endpoint.accept().await {
        let config = config.clone();
        tokio::spawn(async move {
            if let Err(e) = handle(incoming, config).await {
                eprintln!("connection failed: {e:#}");
            }
        });
    }
    registration.abort();
    Ok(())
}

/// Keep a connection to the parent for as long as we can, reconnecting when it
/// drops. The connection *is* the registration.
async fn register(config: Config, home: std::path::PathBuf, listening: Vec<String>) -> Result<()> {
    let keys = keystore(&home);
    let key = id52::decode(&config.key)?;

    loop {
        let secret = keys.secret(&key)?;
        match sirji::daemon::register_device(&secret, &config.parent, &config.parent_hints, &listening)
            .await
        {
            Ok(()) => println!("parent connection closed; reconnecting"),
            Err(e) => eprintln!("cannot reach parent: {e:#}"),
        }
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
}

/// Serve one peer. sirji has already established who they are; everything from
/// here is our own protocol.
async fn handle(incoming: sirji::Incoming, config: std::sync::Arc<Config>) -> Result<()> {
    let conn = incoming.await?;
    let caller = id52::encode(&conn.remote_id());
    println!("connection from {caller}");

    let (mut send, recv) = conn.accept_bi().await?;
    let mut recv = BufReader::new(recv);

    // The knock carries the ticket our parent issued. Verifying it tells us who is
    // calling; we keep no identity state and could not know otherwise.
    let mut line = String::new();
    recv.read_line(&mut line).await?;
    let knock: proto::Knock = serde_json::from_str(line.trim())
        .unwrap_or(proto::Knock { ticket: None });

    match &knock.ticket {
        Some(ticket) => match ticket.verify(&conn.remote_id(), &config.parent) {
            Ok(()) => {
                let who = ticket.alias.as_deref().unwrap_or("an unnamed peer");
                println!("  ticket ok: {who}, for `{}`", ticket.name);
                if ticket.name != config.name {
                    reply(
                        &mut send,
                        &proto::Say::No {
                            reason: format!("this ticket is for `{}`, we are `{}`", ticket.name, config.name),
                        },
                    )
                    .await?;
                    send.finish()?;
                    conn.closed().await;
                    return Ok(());
                }
            }
            Err(e) => {
                println!("  ticket refused: {e:#}");
                reply(&mut send, &proto::Say::No { reason: format!("{e:#}") }).await?;
                send.finish()?;
                conn.closed().await;
                return Ok(());
            }
        },
        None => {
            // Refused, deliberately. A device id52 is not a secret — it is handed
            // to everyone who resolves the name — so accepting un-ticketed dials
            // would make the ticket decorative and the directory readable by
            // anyone who ever looked us up.
            println!("  refused: no ticket");
            reply(
                &mut send,
                &proto::Say::No {
                    reason: "no ticket — resolve us as `name@peer` instead of dialling directly"
                        .into(),
                },
            )
            .await?;
            // Finish and wait: a refusal the caller never receives is
            // indistinguishable from a crash, and sends them debugging the wrong
            // thing.
            send.finish()?;
            conn.closed().await;
            return Ok(());
        }
    }

    let mut line = String::new();
    recv.read_line(&mut line).await?;

    let ask: proto::Ask = match serde_json::from_str(line.trim()) {
        Ok(ask) => ask,
        Err(e) => {
            reply(&mut send, &proto::Say::No { reason: format!("unreadable: {e}") }).await?;
            return Ok(());
        }
    };

    match ask {
        proto::Ask::List { path } => match list(&config, &path) {
            Ok(entries) => {
                println!("  list {path:?} -> {} entries", entries.len());
                reply(&mut send, &proto::Say::Listing { entries }).await?;
            }
            Err(e) => reply(&mut send, &proto::Say::No { reason: format!("{e:#}") }).await?,
        },
        proto::Ask::Get { path } => match config.resolve(&path) {
            Ok(real) if real.is_file() => {
                let bytes = std::fs::read(&real)?;
                let name = real
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.clone());
                println!("  get {path:?} -> {} bytes", bytes.len());
                reply(
                    &mut send,
                    &proto::Say::File { name, bytes: bytes.len() as u64 },
                )
                .await?;
                send.write_all(&bytes).await?;
            }
            Ok(_) => {
                reply(&mut send, &proto::Say::No { reason: format!("{path} is not a file") })
                    .await?
            }
            Err(e) => reply(&mut send, &proto::Say::No { reason: format!("{e:#}") }).await?,
        },
    }

    send.finish()?;
    conn.closed().await;
    Ok(())
}

fn list(config: &Config, path: &str) -> Result<Vec<proto::Entry>> {
    let dir = config.resolve(if path.is_empty() { "." } else { path })?;
    if !dir.is_dir() {
        bail!("{path} is not a directory");
    }
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let meta = entry.metadata()?;
        entries.push(proto::Entry {
            name: entry.file_name().to_string_lossy().to_string(),
            dir: meta.is_dir(),
            bytes: if meta.is_dir() { 0 } else { meta.len() },
        });
    }
    entries.sort_by(|a, b| (b.dir, &a.name).cmp(&(a.dir, &b.name)));
    Ok(entries)
}

async fn reply(
    send: &mut sirji::SendStream,
    say: &proto::Say,
) -> Result<()> {
    let mut text = serde_json::to_string(say)?;
    text.push('\n');
    send.write_all(text.as_bytes()).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// the client side
// ---------------------------------------------------------------------------

/// Dial another t-sirji-fs and ask it one thing.
///
/// We dial from a key of our own, freshly minted here. In the finished design the
/// address comes from resolving `name@peer` at the peer's sirji, which also hands
/// over a ticket; this dials the device key directly, which is the same wire
/// without the doorman.
/// Turn `name@peer` into a device id52 and a ticket, by asking our parent.
///
/// We hold no `network.toml` and so have no idea who `peer` is — that is the
/// point of a device. Our parent knows, asks them on our behalf, and the ticket
/// comes back bound to *our* key, because we are the one who will dial.
async fn resolve(target: &str, key: &str, config: &Config) -> Result<(String, sirji::Ticket, Vec<String>)> {
    let Some((name, alias)) = target.split_once('@') else {
        bail!("{target:?} is not name@peer");
    };
    let home = Config::home()?;
    let secret = keystore(&home).secret(&id52::decode(key)?)?;

    let endpoint = sirji::endpoint::bind_dialer(secret).await?;
    let say = sirji::daemon::ask_as_device(
        &endpoint,
        &config.parent,
        &config.parent_hints,
        &sirji::proto::Ask::ResolveFor {
            name: name.to_string(),
            alias: alias.to_string(),
        },
    )
    .await;
    endpoint.close().await;

    match say? {
        sirji::proto::Say::Resolved { device, ticket, hints } => Ok((device, ticket, hints)),
        sirji::proto::Say::No { reason } => bail!("{reason}"),
    }
}

async fn ask(
    target: &str,
    ask: &proto::Ask,
) -> Result<(proto::Say, BufReader<sirji::RecvStream>)> {
    let home = Config::home()?;
    let keys = keystore(&home);

    // Dial as ourselves when we have an identity, so the ticket can be bound to
    // it. Without a config there is nothing to be bound to, so mint one.
    let (key, ticket, hints) = match Config::load(&home) {
        Ok(config) if target.contains('@') => {
            let (device, ticket, hints) = resolve(target, &config.key, &config).await?;
            eprintln!("resolved {target} -> {device}");
            (device, Some(ticket), hints)
        }
        Ok(config) => (target.to_string(), None, config.parent_hints.clone()),
        Err(_) if target.contains('@') => bail!("no device home: run `t-sirji-fs init` first"),
        Err(_) => (target.to_string(), None, Vec::new()),
    };
    let mine = match Config::load(&home) {
        Ok(config) => id52::decode(&config.key)?,
        Err(_) => keys.generate()?,
    };
    let secret = keys.secret(&mine)?;
    let target = id52::decode(&key)?;

    let endpoint = sirji::endpoint::bind_dialer(secret).await?;
    let conn = match sirji::dial(&endpoint, target).await {
        Ok(conn) => conn,
        Err(direct) => {
            let mut out = Err(direct);
            for hint in &hints {
                if let Ok(socket) = hint.parse()
                    && let Ok(conn) = sirji::endpoint::dial_at(&endpoint, target, socket).await
                {
                    out = Ok(conn);
                    break;
                }
            }
            out?
        }
    };
    let (mut send, recv) = conn.open_bi().await?;

    // The ticket goes first when we have one: it is what lets the device know who
    // we are without holding any identity state of its own.
    let mut text = serde_json::to_string(&proto::Knock { ticket })?;
    text.push('\n');
    send.write_all(text.as_bytes()).await?;

    let mut text = serde_json::to_string(ask)?;
    text.push('\n');
    send.write_all(text.as_bytes()).await?;
    send.finish()?;

    let mut recv = BufReader::new(recv);
    let mut line = String::new();
    recv.read_line(&mut line).await?;
    let say: proto::Say = serde_json::from_str(line.trim())?;

    // Return the reader itself, not its inner stream: reading the header line
    // buffered whatever followed it, and `into_inner` would throw that away — so
    // the file content would vanish between the header and the read.
    Ok((say, recv))
}

async fn client_ls(target: &str, path: &str) -> Result<()> {
    let (say, _) = ask(target, &proto::Ask::List { path: path.to_string() }).await?;
    match say {
        proto::Say::Listing { entries } => {
            if entries.is_empty() {
                println!("(empty)");
            }
            for e in entries {
                if e.dir {
                    println!("{:>10}  {}/", "dir", e.name);
                } else {
                    println!("{:>10}  {}", e.bytes, e.name);
                }
            }
            Ok(())
        }
        proto::Say::No { reason } => bail!("{reason}"),
        other => bail!("unexpected reply: {other:?}"),
    }
}

async fn client_get(target: &str, path: &str) -> Result<()> {
    use tokio::io::AsyncReadExt;

    let (say, mut recv) = ask(target, &proto::Ask::Get { path: path.to_string() }).await?;
    match say {
        proto::Say::File { name, bytes } => {
            // Read exactly what was announced: the header said how much, so we do
            // not have to guess where the content ends.
            let mut buf = vec![0u8; bytes as usize];
            recv.read_exact(&mut buf).await?;
            eprintln!("{name}: {bytes} bytes");
            use std::io::Write;
            std::io::stdout().write_all(&buf)?;
            Ok(())
        }
        proto::Say::No { reason } => bail!("{reason}"),
        other => bail!("unexpected reply: {other:?}"),
    }
}
