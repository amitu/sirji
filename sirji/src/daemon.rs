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
use crate::proto::{
    AddressInfo, Ask, Hello, Invite, PeerInfo, RelayInfo, Request, Response, Say, Welcome,
};
use crate::ticket::Ticket;
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
    /// Device names holding a connection to us right now.
    ///
    /// Not persisted, and deliberately not a timestamped roster: liveness is the
    /// connection, so it lives and dies with the process that holds it.
    live: Mutex<std::collections::BTreeMap<String, Vec<String>>>,
}

impl Daemon {
    /// Bind every non-retired handshake key and start serving.
    pub async fn start(home: PathBuf) -> Result<Arc<Self>> {
        // Operational settings belong to this home, so load them before anything
        // binds an endpoint.
        crate::Settings::load(&home)?.activate();

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
            live: Mutex::new(Default::default()),
        }))
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    /// Home relay state for one bound address.
    ///
    /// Includes relays that have no status yet. A configured relay that has not
    /// connected reports nothing at all through iroh, so showing only what iroh
    /// reports makes a mistyped URL and a working default look identical — both
    /// print no relay line, and "did my config take?" becomes unanswerable.
    fn relays_of(&self, alias: &str) -> Vec<RelayInfo> {
        use iroh::Watcher;
        let mut relays: Vec<RelayInfo> = self
            .bound
            .iter()
            .find(|(a, _)| a == alias)
            .map(|(_, endpoint)| {
                endpoint
                    .home_relay_status()
                    .get()
                    .into_iter()
                    .map(|status| RelayInfo {
                        url: status.url().to_string(),
                        connected: status.is_connected(),
                        error: status.last_error().map(|e| error_chain(e)),
                    })
                    .collect()
            })
            .unwrap_or_default();

        for configured in crate::Settings::active().relay {
            let known = relays
                .iter()
                .any(|r| r.url.trim_end_matches('/') == configured.trim_end_matches('/'));
            if !known {
                relays.push(RelayInfo {
                    url: configured,
                    connected: false,
                    error: Some("configured, no connection yet".into()),
                });
            }
        }
        relays
    }

    /// Where we are reachable right now, as socket addresses, across every address
    /// we listen on.
    async fn hints(&self) -> Vec<String> {
        let mut all = std::collections::BTreeSet::new();
        for (_, endpoint) in &self.bound {
            all.extend(crate::endpoint::reachable_at(endpoint).await);
        }
        all.into_iter().collect()
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
                        match daemon.serve_peer(incoming, &alias).await {
                            Ok(()) => {}
                            // A peer that got what it came for and hung up is not
                            // a failure, and logging it as one trains the operator
                            // to ignore the log.
                            Err(e) if is_clean_close(&e) => {}
                            Err(e) => eprintln!("peer connection failed: {e:#}"),
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

        // A known device claiming its name is the one case where the connection is
        // the point: it stays open, and that is what "live" means.
        let device = {
            let net = self.net.lock().await;
            net.device_by_key(&caller).map(|d| d.name.clone())
        };
        if let Hello::Device { hints } = &hello
            && let Some(name) = device
        {
            let hints = hints.clone();
            let welcome = Welcome::Ok {
                alias: name.clone(),
                addresses: Vec::new(),
                dns: Vec::new(),
            };
            let mut text = serde_json::to_string(&welcome)?;
            text.push(crate::proto::NEWLINE as char);
            send.write_all(text.as_bytes()).await?;

            self.live.lock().await.insert(name.clone(), hints);
            println!("device {name} registered ({caller})");

            // The same connection carries requests. A device that only registers
            // sends none and simply holds the stream open; when it drops — process
            // gone, network gone, cable pulled — the device stops being live with
            // nobody timing anything out.
            let served = self
                .serve_asks(&mut recv, &mut send, &Asker::Device(caller.clone()))
                .await;
            self.live.lock().await.remove(&name);
            println!("device {name} gone");
            if let Err(e) = served {
                eprintln!("device {name}: {e:#}");
            }
            return Ok(());
        }

        let welcome = self.greet(&caller, on, hello).await;
        let mut text = serde_json::to_string(&welcome)?;
        text.push(crate::proto::NEWLINE as char);
        send.write_all(text.as_bytes()).await?;

        if let Welcome::Ok { alias, .. } = &welcome {
            let asker = Asker::Peer(alias.clone());
            self.serve_asks(&mut recv, &mut send, &asker).await?;
        }
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

            // Handled before we got here when the key is known; reaching this means
            // it is not one of our devices.
            Hello::Device { .. } => bail!("{caller} is not one of our devices"),

            Hello::Invited { invited_to, addresses, dns, hints } => {
                // One mechanism, two outcomes. The invite identity says which:
                // a pending device enrols, a pending peer pairs.
                if let Some(device) = net.pending_device_by_invite(&invited_to) {
                    let name = device.name.clone();
                    let index = net
                        .devices
                        .iter()
                        .position(|d| d.name == name)
                        .expect("just found it");
                    net.devices[index].keys = vec![caller.to_string()];
                    net.devices[index].invite = None;
                    net.check()?;
                    net.save(&self.home)?;

                    println!("device {name} enrolled ({caller})");
                    return Ok(Welcome::Ok {
                        alias: name,
                        addresses: net.current_addresses(),
                        dns: net.dns.clone(),
                    });
                }

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
                net.peers[index].hints = hints;
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

    /// Read requests until the stream ends.
    ///
    /// Who is asking decides what they may ask for: a peer wants one of our
    /// devices located, one of our own devices wants us to locate someone else's.
    async fn serve_asks(
        &self,
        recv: &mut BufReader<iroh::endpoint::RecvStream>,
        send: &mut iroh::endpoint::SendStream,
        asker: &Asker,
    ) -> Result<()> {
        loop {
            let mut line = String::new();
            if recv.read_line(&mut line).await? == 0 {
                return Ok(()); // the peer hung up: normal
            }
            if line.trim().is_empty() {
                continue;
            }
            let ask: Ask = match serde_json::from_str(line.trim()) {
                Ok(ask) => ask,
                Err(e) => {
                    reply(send, &Say::No { reason: format!("unreadable: {e}") }).await?;
                    continue;
                }
            };

            let say = match self.answer(ask, asker).await {
                Ok(say) => say,
                Err(e) => Say::No { reason: format!("{e:#}") },
            };
            reply(send, &say).await?;
        }
    }

    async fn answer(&self, ask: Ask, asker: &Asker) -> Result<Say> {
        match ask {
            // A peer asking where one of our devices is.
            Ask::Resolve { name, caller } => {
                let Asker::Peer(alias) = asker else {
                    bail!("only a peer may ask us to locate our devices");
                };
                self.locate(&name, &caller, Some(alias.clone())).await
            }

            // One of our own devices looking for a sibling. They hold no
            // network.toml and cannot find each other any other way, and asking a
            // peer about something inside our own fleet would be absurd. The
            // answer includes a ticket: being a device of ours is not a reason to
            // skip authorisation.
            Ask::ResolveLocal { name } => {
                let Asker::Device(caller) = asker else {
                    bail!("only our own devices may resolve a sibling");
                };
                self.locate(&name, caller, None).await
            }

            // One of our own devices asking us to resolve a name at a peer. It
            // holds no network.toml, so it cannot know who `alias` is; we do.
            Ask::ResolveFor { name, alias } => {
                let Asker::Device(caller) = asker else {
                    bail!("only our own devices may ask us to resolve for them");
                };
                let peer = {
                    let net = self.net.lock().await;
                    net.peer_by_alias(&alias)
                        .cloned()
                        .ok_or_else(|| anyhow::anyhow!("we know nobody called {alias:?}"))?
                };
                let mine = self.keys.secret(&id52::decode(&peer.mine)?)?;

                // Dial them as the identity they know us by, and ask on the
                // device's behalf: the ticket must be bound to the key that will
                // actually dial, which is the device's, not ours.
                let endpoint = crate::endpoint::bind_dialer(mine).await?;
                let result = ask_peer(
                    &endpoint,
                    &peer.addresses,
                    &peer.hints,
                    &Ask::Resolve { name, caller: caller.to_string() },
                )
                .await;
                endpoint.close().await;
                result
            }
        }
    }

    /// Locate one of our devices and mint a ticket admitting `caller` to it.
    ///
    /// `alias` is the person asking, when there is one. A sibling device gets
    /// `None`: there is no person behind it to name.
    async fn locate(&self, name: &str, caller: &str, alias: Option<String>) -> Result<Say> {
        let device_key = {
            let net = self.net.lock().await;
            let Some(entry) = net.device_by_name(name) else {
                bail!("we have no device called {name:?}");
            };
            let Some(key) = entry.keys.first().cloned() else {
                bail!("{name:?} has not enrolled yet");
            };
            key
        };

        // Only a live device is worth returning: handing back an address nobody is
        // listening on turns one clear failure into a dial timeout. The value is
        // where the device told us it listens, which is what lets a caller reach it
        // with no discovery service involved at all.
        let Some(hints) = self.live.lock().await.get(name).cloned() else {
            bail!("{name:?} is not connected");
        };

        // Sign with one of our current addresses. A device knows its parent's
        // handshake keys, so it can check the issuer is genuinely us rather than
        // trusting whatever the caller presents.
        let issuer = {
            let net = self.net.lock().await;
            let key = net
                .current_addresses()
                .first()
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("we have no current address to sign with"))?;
            self.keys.secret(&id52::decode(&key)?)?
        };

        id52::decode(caller)?;
        let ticket = Ticket::mint(&issuer, name, caller, alias, crate::ticket::LIFETIME_SECS);
        println!("resolved {name} for {caller}");
        Ok(Say::Resolved { device: device_key, ticket, hints })
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
                            relays: self.relays_of(&k.alias),
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
                    // Filled in when they accept and tell us where they are.
                    hints: vec![],
                    reached_on: None,
                });
                net.check()?;
                net.save(&self.home)?;

                Ok(Response::Invite {
                    invite: Invite {
                        addresses: net.current_addresses(),
                        dns: net.dns.clone(),
                        identity: mine,
                        hints: self.hints().await,
                    },
                })
            }

            Request::DeviceInvite { name } => {
                let mut net = self.net.lock().await;
                if net.device_by_name(&name).is_some() {
                    bail!("we already have a device called {name:?}");
                }
                let invite = id52::encode(&self.keys.generate()?);
                net.devices.push(crate::config::Device {
                    name,
                    keys: Vec::new(),
                    invite: Some(invite.clone()),
                });
                net.check()?;
                net.save(&self.home)?;

                Ok(Response::Invite {
                    invite: Invite {
                        addresses: net.current_addresses(),
                        dns: net.dns.clone(),
                        identity: invite,
                        hints: self.hints().await,
                    },
                })
            }

            Request::Devices => {
                let net = self.net.lock().await;
                let live = self.live.lock().await;
                Ok(Response::Devices {
                    devices: net
                        .devices
                        .iter()
                        .map(|d| crate::proto::DeviceInfo {
                            name: d.name.clone(),
                            keys: d.keys.clone(),
                            pending: d.is_pending(),
                            live: live.contains_key(&d.name),
                        })
                        .collect(),
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
            hints: self.hints().await,
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
                                // Where we just reached them. Current by
                                // construction: the dial that got us here used it.
                                hints: invite.hints.clone(),
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

async fn reply(send: &mut iroh::endpoint::SendStream, say: &Say) -> Result<()> {
    let mut text = serde_json::to_string(say)?;
    text.push(crate::proto::NEWLINE as char);
    send.write_all(text.as_bytes()).await?;
    Ok(())
}

/// Dial a peer as an established relationship and ask them one thing.
pub async fn ask_peer(
    endpoint: &Endpoint,
    addresses: &[String],
    hints: &[String],
    ask: &Ask,
) -> Result<Say> {
    let mut last = None;
    for address in addresses {
        let target = id52::decode(address)?;
        let conn = match dial_with_hints(endpoint, target, hints).await {
            Ok(conn) => Ok(conn),
            Err(hint_err) => crate::dial(endpoint, target)
                .await
                .map_err(|discovery_err| hint_err.context(discovery_err)),
        };
        match conn {
            Ok(conn) => {
                let out = converse(&conn, &Hello::Peer, ask).await;
                conn.close(0u32.into(), b"done");
                match out {
                    Ok(say) => return Ok(say),
                    Err(e) => last = Some(e),
                }
            }
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("no addresses to try")))
}

/// Dial our parent as one of its devices and ask it one thing.
///
/// A device holds no `network.toml`, so anything requiring knowledge of who a peer
/// is has to go through the parent. This is that channel.
pub async fn ask_as_device(
    endpoint: &Endpoint,
    addresses: &[String],
    hints: &[String],
    ask: &Ask,
) -> Result<Say> {
    let mut last = None;
    for address in addresses {
        let target = id52::decode(address)?;
        let conn = match dial_with_hints(endpoint, target, hints).await {
            Ok(conn) => Ok(conn),
            Err(hint_err) => crate::dial(endpoint, target)
                .await
                .map_err(|discovery_err| hint_err.context(discovery_err)),
        };
        match conn {
            Ok(conn) => {
                let out = converse(&conn, &Hello::Device { hints: Vec::new() }, ask).await;
                conn.close(0u32.into(), b"done");
                match out {
                    Ok(say) => return Ok(say),
                    Err(e) => last = Some(e),
                }
            }
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("no parent addresses to try")))
}

/// Greet as a device, then keep the stream open until the connection ends.
///
/// The streams are held in scope for the whole wait, deliberately. Two ways to get
/// this wrong, and both were: calling `finish()` on the send half, and simply
/// letting it drop at the end of a helper — either closes the stream, and the
/// parent reads end-of-stream as the device having departed. So a device would
/// report its own disappearance the instant it arrived, forever.
async fn hold(conn: &crate::Connection, listening_on: &[String]) -> Result<()> {
    let (mut send, recv) = conn.open_bi().await?;
    let mut recv = BufReader::new(recv);

    let hello = Hello::Device {
        hints: listening_on.to_vec(),
    };
    send.write_all(format!("{}\n", serde_json::to_string(&hello)?).as_bytes())
        .await?;

    let mut line = String::new();
    recv.read_line(&mut line).await?;
    match serde_json::from_str(line.trim())? {
        Welcome::Ok { alias, .. } => println!("registered with parent as `{alias}`"),
        Welcome::No { reason } => bail!("the parent refused us: {reason}"),
    }

    conn.closed().await;
    drop(send);
    drop(recv);
    Ok(())
}

/// Greet, then ask, on one connection.
async fn converse(conn: &crate::Connection, hello: &Hello, ask: &Ask) -> Result<Say> {
    let (mut send, recv) = conn.open_bi().await?;
    let mut recv = BufReader::new(recv);

    for line in [serde_json::to_string(hello)?, serde_json::to_string(ask)?] {
        send.write_all(format!("{line}\n").as_bytes()).await?;
    }

    let mut greeting = String::new();
    recv.read_line(&mut greeting).await?;
    if let Welcome::No { reason } = serde_json::from_str(greeting.trim())? {
        bail!("refused: {reason}");
    }

    let mut answer = String::new();
    recv.read_line(&mut answer).await?;
    Ok(serde_json::from_str(answer.trim())?)
}

/// The whole error, not just its outermost sentence.
///
/// "Failed to connect to relay server" names a symptom; the cause is three
/// `source()` hops down, and without it a network problem is undiagnosable.
fn error_chain(error: &dyn std::error::Error) -> String {
    let mut out = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        out.push_str(&format!(": {cause}"));
        source = cause.source();
    }
    out
}

/// Did this connection simply end, rather than break?
fn is_clean_close(error: &anyhow::Error) -> bool {
    let text = format!("{error:#}");
    text.contains("closed by peer") || text.contains("connection closed")
}

/// Who is on the other end of a connection we have already greeted.
enum Asker {
    /// Another person's sirji, known to us by this alias.
    Peer(String),
    /// One of our own devices, by its key.
    Device(String),
}

/// Note which of our addresses a peer arrived on. This is what makes retiring an
/// address decidable instead of a guess.
fn note_reached_on(net: &mut Network, alias: &str, on: &str) {
    if let Some(peer) = net.peers.iter_mut().find(|p| p.alias == alias) {
        peer.reached_on = Some(on.to_string());
    }
}

/// Try the remembered addresses, then give up so the caller can fall back to
/// discovery.
async fn dial_with_hints(
    endpoint: &Endpoint,
    target: crate::PublicKey,
    hints: &[String],
) -> Result<crate::Connection> {
    crate::endpoint::dial_hints(endpoint, target, hints).await
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

/// Dial a parent listed in an invite and exchange one `Hello`.
///
/// Used by a device enrolling itself. Tries the invite's socket hints first, then
/// falls back to dialling by key, exactly as pairing does.
pub async fn greet_parent(
    endpoint: &Endpoint,
    invite: &Invite,
    hello: Hello,
) -> Result<Welcome> {
    let mut last = None;
    for address in &invite.addresses {
        let target = id52::decode(address)?;
        let conn = match dial_with_hints(endpoint, target, &invite.hints).await {
            Ok(conn) => Ok(conn),
            Err(hint_err) => crate::dial(endpoint, target)
                .await
                .map_err(|discovery_err| hint_err.context(discovery_err)),
        };
        match conn {
            Ok(conn) => {
                let welcome = exchange(&conn, &hello).await;
                conn.close(0u32.into(), b"done");
                match welcome {
                    Ok(welcome) => return Ok(welcome),
                    Err(e) => last = Some(e),
                }
            }
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("the invite carried no addresses")))
}

/// Register a device with its parent and **hold the connection** until it drops.
///
/// The connection is the registration: while it is open the parent counts this
/// device as live, and when it closes the parent stops. Returning `Ok` means the
/// parent hung up, not that anything went wrong.
pub async fn register_device(
    secret: &crate::SecretKey,
    addresses: &[String],
    hints: &[String],
    listening_on: &[String],
) -> Result<()> {
    let endpoint = crate::endpoint::bind_dialer(secret.clone()).await?;

    let mut last = None;
    for address in addresses {
        let target = id52::decode(address)?;
        let conn = match dial_with_hints(&endpoint, target, hints).await {
            Ok(conn) => Ok(conn),
            Err(hint_err) => crate::dial(&endpoint, target)
                .await
                .map_err(|discovery_err| hint_err.context(discovery_err)),
        };
        match conn {
            Ok(conn) => {
                // Deliberately not `exchange`: that finishes the send stream, and
                // a finished stream is indistinguishable from a departed device.
                // Registration has to leave it open, because the open stream *is*
                // the registration.
                match hold(&conn, listening_on).await {
                    Ok(()) => {
                        endpoint.close().await;
                        return Ok(());
                    }
                    Err(e) => last = Some(e),
                }
            }
            Err(e) => last = Some(e),
        }
    }
    endpoint.close().await;
    Err(last.unwrap_or_else(|| anyhow::anyhow!("no parent addresses to try")))
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
        format!("no daemon at {} — start one with `sirji daemon`", path.display())
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
