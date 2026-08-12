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

  t-sirji-fs ls  <id52> [path]
  t-sirji-fs get <id52> <path>
        read from another t-sirji-fs. Dials it directly for now; resolving
        `name@peer` through the parent is the next step.

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
    let registration = tokio::spawn(register(config.clone(), home.clone()));

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
async fn register(config: Config, home: std::path::PathBuf) -> Result<()> {
    let keys = keystore(&home);
    let key = id52::decode(&config.key)?;

    loop {
        let secret = keys.secret(&key)?;
        match sirji::daemon::register_device(&secret, &config.parent, &config.parent_hints).await {
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
    send: &mut iroh::endpoint::SendStream,
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
async fn ask(
    target: &str,
    ask: &proto::Ask,
) -> Result<(proto::Say, BufReader<iroh::endpoint::RecvStream>)> {
    let target = id52::decode(target)?;
    let home = Config::home()?;
    let keys = keystore(&home);
    let secret = keys.secret(&keys.generate()?)?;

    let endpoint = sirji::endpoint::bind_dialer(secret).await?;
    let conn = sirji::dial(&endpoint, target).await?;
    let (mut send, recv) = conn.open_bi().await?;

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
