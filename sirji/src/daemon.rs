//! `sirjid` — the daemon. One per `$SIRJI_HOME`.
//!
//! It binds the handshake keys, accepts peer connections, and serves the CLI over
//! a unix socket in the home directory. It is the only thing that touches the
//! network; the CLI does no networking of its own.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use iroh::Endpoint;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Mutex;

use crate::config::{HandshakeKey, Network, Peer};
use crate::proto::{AddressInfo, Hello, Invite, PeerInfo, Request, Response, Welcome};
use crate::{Keystore, id52};

pub const SOCKET: &str = "sirji.sock";

/// Everything the daemon owns, shared between the peer listeners and the control
/// socket. The `network.toml` lock is what keeps a pairing in flight from racing
/// a CLI command.
pub struct Daemon {
    home: PathBuf,
    keys: Keystore,
    net: Mutex<Network>,
    /// One endpoint per handshake key, because a key you can be reached at is a
    /// bound endpoint. Peer keys are not here: they are dialled from, never
    /// listened on.
    bound: Vec<(String, Endpoint)>,
}

impl Daemon {
    /// Bind every non-retired handshake key and start serving.
    pub async fn start(home: PathBuf) -> Result<Arc<Self>> {
        let keys = Keystore::at(home.join("keys"));
        let net = Network::load(&home)?;
        net.check().context("network.toml is not usable")?;

        let mut bound = Vec::new();
        for hk in &net.handshake_keys {
            if hk.retired {
                // Retired keys stay bound until every peer has moved off them.
                // Not skipping them is the point of retirement-by-draining.
            }
            let key = id52::decode(&hk.key)?;
            let secret = keys.secret(&key)?;
            let endpoint = crate::bind(secret).await?;
            println!("listening on {} ({})", hk.alias, hk.key);
            bound.push((hk.alias.clone(), endpoint));
        }
        if bound.is_empty() {
            bail!("no handshake keys in network.toml — run `sirji init` first");
        }

        Ok(Arc::new(Self {
            home,
            keys,
            net: Mutex::new(net),
            bound,
        }))
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    /// Where we are reachable right now, as socket addresses.
    ///
    /// Sockets bind to the unspecified address, so the port is the real
    /// information; loopback covers another instance on this machine, which is
    /// the case discovery cannot help with anyway when it is unavailable.
    fn hints(&self) -> Vec<String> {
        self.bound
            .iter()
            .flat_map(|(_, endpoint)| endpoint.bound_sockets())
            .map(|addr| addr.port())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .map(|port| format!("127.0.0.1:{port}"))
            .collect()
    }

    /// Accept peers on every bound key, and CLI commands on the socket, until
    /// something goes wrong.
    pub async fn run(self: Arc<Self>) -> Result<()> {
        for (alias, endpoint) in &self.bound {
            let daemon = self.clone();
            let endpoint = endpoint.clone();
            let alias = alias.clone();
            tokio::spawn(async move {
                while let Some(incoming) = endpoint.accept().await {
                    let daemon = daemon.clone();
                    let alias = alias.clone();
                    tokio::spawn(async move {
                        if let Err(e) = daemon.serve_peer(incoming, &alias).await {
                            eprintln!("peer connection failed: {e:#}");
                        }
                    });
                }
            });
        }
        self.serve_control().await
    }

    // -- peers ---------------------------------------------------------------

    /// A connection arrived on one of our addresses. Who it is was settled by the
    /// transport before we saw a byte: look the dialling key up, and the answer
    /// decides everything.
    async fn serve_peer(self: Arc<Self>, incoming: crate::Incoming, on: &str) -> Result<()> {
        let conn = incoming.await?;
        let caller = id52::encode(&conn.remote_id());

        let (mut send, recv) = conn.accept_bi().await?;
        let mut recv = BufReader::new(recv);
        let mut line = String::new();
        recv.read_line(&mut line).await?;
        let hello: Hello = serde_json::from_str(line.trim())
            .with_context(|| format!("unreadable hello from {caller}"))?;

        let welcome = self.greet(&caller, on, hello).await;
        let mut text = serde_json::to_string(&welcome)?;
        text.push(crate::proto::NEWLINE as char);
        send.write_all(text.as_bytes()).await?;
        send.finish()?;
        conn.closed().await;
        Ok(())
    }

    async fn greet(&self, caller: &str, on: &str, hello: Hello) -> Welcome {
        match self.greet_inner(caller, on, hello).await {
            Ok(w) => w,
            Err(e) => Welcome::No {
                reason: format!("{e:#}"),
            },
        }
    }

    async fn greet_inner(&self, caller: &str, on: &str, hello: Hello) -> Result<Welcome> {
        let mut net = self.net.lock().await;

        // Known key: an existing relationship, whatever it says it is.
        if let Some(peer) = net.peer_by_key(caller) {
            let alias = peer.alias.clone();
            println!("peer {alias} connected on {on}");
            note_reached_on(&mut net, &alias, on);
            let (addresses, dns) = (net.current_addresses(), net.dns.clone());
            net.save(&self.home)?;
            return Ok(Welcome::Ok { alias, addresses, dns });
        }

        match hello {
            Hello::Peer => bail!("we have no relationship with {caller}"),

            Hello::Invited { invited_to, addresses, dns } => {
                // The key we minted for them went to exactly one person, so
                // presenting it is the proof of being that person.
                let Some(pending) = net.pending_by_mine(&invited_to) else {
                    bail!("no invite outstanding for that identity");
                };
                let alias = pending.alias.clone();

                let index = net
                    .peers
                    .iter()
                    .position(|p| p.alias == alias)
                    .expect("just found it");
                net.peers[index].peer = Some(caller.to_string());
                net.peers[index].addresses = addresses;
                net.peers[index].dns = dns;
                net.peers[index].reached_on = Some(on.to_string());
                net.check()?;
                net.save(&self.home)?;

                println!("paired with {alias} ({caller})");
                Ok(Welcome::Ok {
                    alias,
                    addresses: net.current_addresses(),
                    dns: net.dns.clone(),
                })
            }
        }
    }

    // -- control socket ------------------------------------------------------

    async fn serve_control(self: Arc<Self>) -> Result<()> {
        let path = self.home.join(SOCKET);
        // Unix socket paths are capped by the kernel (104 bytes on macOS, 108 on
        // Linux) and the failure otherwise reads as "path must be shorter than
        // SUN_LEN", which says nothing about what to do.
        const SUN_LEN: usize = 100;
        if path.as_os_str().len() >= SUN_LEN {
            bail!(
                "the control socket path is {} bytes, over the {SUN_LEN}-byte limit \
                 the kernel allows: {}\nuse a shorter $SIRJI_HOME.",
                path.as_os_str().len(),
                path.display()
            );
        }
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)
            .with_context(|| format!("binding {}", path.display()))?;
        println!("control socket at {}", path.display());

        loop {
            let (stream, _) = listener.accept().await?;
            let daemon = self.clone();
            tokio::spawn(async move {
                if let Err(e) = daemon.serve_cli(stream).await {
                    eprintln!("cli connection failed: {e:#}");
                }
            });
        }
    }

    async fn serve_cli(self: Arc<Self>, stream: UnixStream) -> Result<()> {
        let (recv, mut send) = stream.into_split();
        let mut recv = BufReader::new(recv);
        let mut line = String::new();
        recv.read_line(&mut line).await?;

        let response = match serde_json::from_str::<Request>(line.trim()) {
            Ok(request) => self.handle(request).await.unwrap_or_else(|e| Response::Error {
                message: format!("{e:#}"),
            }),
            Err(e) => Response::Error {
                message: format!("unreadable request: {e}"),
            },
        };

        let mut text = serde_json::to_string(&response)?;
        text.push(crate::proto::NEWLINE as char);
        send.write_all(text.as_bytes()).await?;
        Ok(())
    }

    async fn handle(&self, request: Request) -> Result<Response> {
        match request {
            Request::Status => {
                let net = self.net.lock().await;
                Ok(Response::Status {
                    home: self.home.display().to_string(),
                    addresses: net
                        .handshake_keys
                        .iter()
                        .map(|k| AddressInfo {
                            alias: k.alias.clone(),
                            key: k.key.clone(),
                            retired: k.retired,
                            bound: self.bound.iter().any(|(a, _)| *a == k.alias),
                        })
                        .collect(),
                    peers: net.peers.iter().filter(|p| !p.is_pending()).count(),
                    pending: net.peers.iter().filter(|p| p.is_pending()).count(),
                })
            }

            Request::Peers => {
                let net = self.net.lock().await;
                Ok(Response::Peers {
                    peers: net
                        .peers
                        .iter()
                        .map(|p| PeerInfo {
                            alias: p.alias.clone(),
                            peer: p.peer.clone(),
                            mine: p.mine.clone(),
                            addresses: p.addresses.clone(),
                            reached_on: p.reached_on.clone(),
                        })
                        .collect(),
                })
            }

            Request::NewAddress { alias } => {
                let mut net = self.net.lock().await;
                if net.handshake_key_by_alias(&alias).is_some() {
                    bail!("we already have a handshake key called {alias:?}");
                }
                let key = self.keys.generate()?;
                let key = id52::encode(&key);
                net.handshake_keys.push(HandshakeKey {
                    alias: alias.clone(),
                    key: key.clone(),
                    retired: false,
                });
                net.save(&self.home)?;
                // Binding it needs a restart; say so rather than pretend.
                Ok(Response::NewAddress { alias, key })
            }

            Request::Invite { alias } => {
                let mut net = self.net.lock().await;
                if net.peer_by_alias(&alias).is_some() {
                    bail!("we already know someone called {alias:?}");
                }
                // Mint the identity they will know us by. Fresh, for them alone.
                let mine = id52::encode(&self.keys.generate()?);
                net.peers.push(Peer {
                    alias,
                    peer: None,
                    mine: mine.clone(),
                    addresses: vec![],
                    dns: vec![],
                    reached_on: None,
                });
                net.check()?;
                net.save(&self.home)?;

                Ok(Response::Invite {
                    invite: Invite {
                        addresses: net.current_addresses(),
                        dns: net.dns.clone(),
                        identity: mine,
                        hints: self.hints(),
                    },
                })
            }

            Request::Accept { alias, invite } => self.accept(alias, invite).await,
        }
    }

    /// Complete someone's invite: mint our identity for them, dial one of their
    /// addresses as that identity, and present what they sent us.
    async fn accept(&self, alias: String, invite: Invite) -> Result<Response> {
        {
            let net = self.net.lock().await;
            if net.peer_by_alias(&alias).is_some() {
                bail!("we already know someone called {alias:?}");
            }
        }

        let mine_key = self.keys.generate()?;
        let mine = id52::encode(&mine_key);
        let secret = self.keys.secret(&mine_key)?;

        let (our_addresses, our_dns) = {
            let net = self.net.lock().await;
            (net.current_addresses(), net.dns.clone())
        };

        let hello = Hello::Invited {
            invited_to: invite.identity.clone(),
            addresses: our_addresses,
            dns: our_dns,
        };

        // Dial from the identity we just minted: this is what they will recognise
        // us by forever, and it is shown to nobody else.
        let endpoint = crate::endpoint::bind_dialer(secret).await?;
        let mut last = None;
        for address in &invite.addresses {
            let target = id52::decode(address)?;
            // Hints first: they are current as of the invite and need no
            // discovery. Fall back to dialling by key alone, which is what
            // endures once the hints go stale.
            let attempt = match dial_with_hints(&endpoint, target, &invite.hints).await {
                Ok(conn) => Ok(conn),
                Err(hint_err) => crate::dial(&endpoint, target)
                    .await
                    .map_err(|discovery_err| hint_err.context(discovery_err)),
            };
            match attempt {
                Ok(conn) => {
                    let welcome = exchange(&conn, &hello).await;
                    conn.close(0u32.into(), b"done");
                    match welcome {
                        Ok(Welcome::Ok { addresses, dns, .. }) => {
                            let mut net = self.net.lock().await;
                            net.peers.push(Peer {
                                alias: alias.clone(),
                                peer: Some(invite.identity.clone()),
                                mine,
                                addresses,
                                dns,
                                reached_on: None,
                            });
                            net.check()?;
                            net.save(&self.home)?;
                            endpoint.close().await;
                            return Ok(Response::Accepted { alias });
                        }
                        Ok(Welcome::No { reason }) => last = Some(anyhow::anyhow!("{reason}")),
                        Err(e) => last = Some(e),
                    }
                }
                Err(e) => last = Some(e),
            }
        }
        endpoint.close().await;
        Err(last.unwrap_or_else(|| anyhow::anyhow!("the invite carried no addresses")))
    }
}

/// Note which of our addresses a peer arrived on. This is what makes retiring an
/// address decidable instead of a guess.
fn note_reached_on(net: &mut Network, alias: &str, on: &str) {
    if let Some(peer) = net.peers.iter_mut().find(|p| p.alias == alias) {
        peer.reached_on = Some(on.to_string());
    }
}

/// Try each hint, then give up so the caller can fall back to discovery.
async fn dial_with_hints(
    endpoint: &Endpoint,
    target: crate::PublicKey,
    hints: &[String],
) -> Result<crate::Connection> {
    let mut last = None;
    for hint in hints {
        let socket = match hint.parse() {
            Ok(s) => s,
            Err(e) => {
                last = Some(anyhow::anyhow!("{hint:?} is not a socket address: {e}"));
                continue;
            }
        };
        match crate::endpoint::dial_at(endpoint, target, socket).await {
            Ok(conn) => return Ok(conn),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("no usable hints")))
}

async fn exchange(conn: &crate::Connection, hello: &Hello) -> Result<Welcome> {
    let (mut send, recv) = conn.open_bi().await?;
    let mut text = serde_json::to_string(hello)?;
    text.push(crate::proto::NEWLINE as char);
    send.write_all(text.as_bytes()).await?;
    send.finish()?;

    let mut recv = BufReader::new(recv);
    let mut line = String::new();
    recv.read_line(&mut line).await?;
    Ok(serde_json::from_str(line.trim())?)
}

/// Create a home directory with its first handshake key.
pub fn init(home: &Path) -> Result<(PathBuf, String)> {
    if Network::path_in(home).exists() {
        bail!("{} already exists", Network::path_in(home).display());
    }
    std::fs::create_dir_all(home)
        .with_context(|| format!("creating {}", home.display()))?;

    let keys = Keystore::at(home.join("keys"));
    let key = id52::encode(&keys.generate()?);

    let net = Network {
        handshake_keys: vec![HandshakeKey {
            alias: "default".into(),
            key: key.clone(),
            retired: false,
        }],
        ..Default::default()
    };
    net.save(home)?;
    Ok((home.to_path_buf(), key))
}

/// Ask the daemon for this home directory to do something.
pub async fn ask(home: &Path, request: &Request) -> Result<Response> {
    let path = home.join(SOCKET);
    let stream = UnixStream::connect(&path).await.with_context(|| {
        format!("no daemon at {} — start one with `sirjid`", path.display())
    })?;
    let (recv, mut send) = stream.into_split();

    let mut text = serde_json::to_string(request)?;
    text.push(crate::proto::NEWLINE as char);
    send.write_all(text.as_bytes()).await?;
    send.shutdown().await?;

    let mut recv = BufReader::new(recv);
    let mut line = String::new();
    recv.read_line(&mut line).await?;
    Ok(serde_json::from_str(line.trim())?)
}
