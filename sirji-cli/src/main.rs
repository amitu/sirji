//! `sirji` — operator commands, and the harness the milestones are demonstrated
//! through.
//!
//! Deliberately hand-rolled argument matching for now; `clap` arrives when there
//! is enough surface to justify it.

use anyhow::{Result, bail};
use sirji::{Keystore, id52};

const USAGE: &str = "\
sirji — peer-to-peer network substrate

  sirji key new                 mint a key, print its id52
  sirji key ls                  list the keystore, verifying every entry
  sirji listen [<id52>]         listen on a key (minting one if not given)
  sirji dial <target> [message]  dial an address and echo a message off it

A <target> is an id52, optionally with a direct socket address appended as
`<id52>@<host>:<port>`. Plain id52 needs discovery to be reachable; the direct
form dials the wire straight and is how the transport is tested where discovery
is unavailable.

The keystore lives at $SIRJI_HOME/keys (default ~/.sirji/keys).";

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();

    match args.as_slice() {
        [] | ["-h"] | ["--help"] | ["help"] => {
            println!("{USAGE}");
            Ok(())
        }
        ["key", "new"] => key_new(),
        ["key", "ls"] => key_ls(),
        ["listen"] => rt()?.block_on(listen(None)),
        ["listen", key] => rt()?.block_on(listen(Some(key))),
        ["dial", address] => rt()?.block_on(dial(address, "hello from sirji")),
        ["dial", address, message @ ..] => rt()?.block_on(dial(address, &message.join(" "))),
        _ => {
            eprintln!("{USAGE}");
            bail!("unrecognised command: {}", args.join(" "));
        }
    }
}

fn rt() -> Result<tokio::runtime::Runtime> {
    Ok(tokio::runtime::Runtime::new()?)
}

fn key_new() -> Result<()> {
    let store = Keystore::open()?;
    let key = store.generate()?;
    println!("{}", id52::encode(&key));
    eprintln!("written to {}", store.dir().display());
    Ok(())
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
    eprintln!("{} key(s) in {}, all verified", keys.len(), store.dir().display());
    Ok(())
}

async fn listen(key: Option<&str>) -> Result<()> {
    let store = Keystore::open()?;
    let key = match key {
        Some(text) => id52::decode(text)?,
        None => store.generate()?,
    };
    let secret = store.secret(&key)?;

    let endpoint = sirji::bind(secret).await?;
    let me = id52::encode(&endpoint.id());
    println!("listening as {me}");

    // Deliberately not gated on `online()`: that waits for discovery to publish,
    // which needs reachable pkarr infrastructure. Accepting works without it, and
    // a direct address is enough to prove the wire.
    for addr in endpoint.bound_sockets() {
        println!("  bound   {addr}");
    }
    // Sockets bind to the unspecified address, so print the port and let the
    // caller supply a reachable host — from another machine that is this box's
    // LAN address, from this one it is 127.0.0.1.
    if let Some(port) = endpoint.bound_sockets().iter().map(|a| a.port()).next() {
        println!("  dial it: sirji dial {me}@<host>:{port}");
    }
    eprintln!("accepting connections");

    while let Some(incoming) = endpoint.accept().await {
        tokio::spawn(async move {
            if let Err(e) = serve(incoming).await {
                eprintln!("connection failed: {e:#}");
            }
        });
    }
    Ok(())
}

async fn serve(incoming: sirji::Incoming) -> Result<()> {
    let conn = incoming.await?;

    // The dialer's identity, established by iroh before a byte of ours moves.
    // This is where the design's known/unknown split will live: a key present in
    // network.toml is an existing relationship, anything else is a handshake.
    let caller = conn.remote_id();
    println!("connection from {}", id52::encode(&caller));

    let (mut send, mut recv) = conn.accept_bi().await?;
    let bytes = tokio::io::copy(&mut recv, &mut send).await?;
    send.finish()?;
    println!("  echoed {bytes} byte(s)");

    conn.closed().await;
    Ok(())
}

async fn dial(target: &str, message: &str) -> Result<()> {
    let (address, direct) = parse_target(target)?;
    let store = Keystore::open()?;

    // We dial from a freshly minted key. In the finished design this is the peer
    // key for this relationship, minted once and kept; here it demonstrates the
    // property that matters — the identity we present is ours to choose, and the
    // listener sees exactly this key.
    let identity = store.generate()?;
    let secret = store.secret(&identity)?;
    println!("dialling as {}", id52::encode(&identity));

    let endpoint = sirji::endpoint::bind_dialer(secret).await?;
    let conn = match direct {
        Some(socket) => sirji::endpoint::dial_at(&endpoint, address, socket).await?,
        None => sirji::dial(&endpoint, address).await?,
    };

    let (mut send, mut recv) = conn.open_bi().await?;
    send.write_all(message.as_bytes()).await?;
    send.finish()?;

    let echoed = recv.read_to_end(64 * 1024).await?;
    println!("echoed back: {}", String::from_utf8_lossy(&echoed));

    conn.close(0u32.into(), b"done");
    endpoint.close().await;
    Ok(())
}

/// `<id52>` or `<id52>@<host>:<port>`.
fn parse_target(target: &str) -> Result<(sirji::PublicKey, Option<std::net::SocketAddr>)> {
    match target.split_once('@') {
        None => Ok((id52::decode(target)?, None)),
        Some((key, socket)) => {
            let socket = socket
                .parse()
                .map_err(|e| anyhow::anyhow!("{socket:?} is not a host:port address: {e}"))?;
            Ok((id52::decode(key)?, Some(socket)))
        }
    }
}

