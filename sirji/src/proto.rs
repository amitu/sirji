//! What sirjis say to each other, and what the CLI says to its daemon.
//!
//! JSON on both, deliberately: neither is throughput-sensitive, and a protocol
//! meant to have a second implementation is better served by a format anyone can
//! read off the wire than by a compact one. Revisit before anything is called
//! stable.

use serde::{Deserialize, Serialize};

/// One JSON value per line, both directions, on every channel.
pub const NEWLINE: u8 = b'\n';

// ---------------------------------------------------------------------------
// peer <-> peer
// ---------------------------------------------------------------------------

/// Opening message on a peer connection. The dialer's key is already known from
/// the transport, so this never carries identity — only what the transport
/// cannot say.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Hello {
    /// An existing relationship. Nothing to prove; `remote_id()` said who this is.
    Peer,

    /// A device of ours, connecting to claim its name and stay reachable.
    ///
    /// There is no heartbeat: the connection **is** the liveness signal. QUIC
    /// already has keepalives and tells us when a peer goes away, so a timer on
    /// top would be reinventing it — and a device that has to be reachable has to
    /// hold a connection anyway.
    Device {
        /// Socket addresses this device is listening on.
        ///
        /// The parent hands these to anyone who resolves the device's name, which
        /// is what makes a device reachable without depending on a discovery
        /// service at all. Hints, not identity: they say *where*, the key still
        /// says *who*.
        #[serde(default)]
        hints: Vec<String>,
    },

    /// First contact, answering an invite. `invited_to` is the key the other side
    /// minted for us and sent out of band — since it went to exactly one person,
    /// presenting it is the proof of being that person.
    Invited {
        invited_to: String,
        /// Where we can be reached, so they can dial us back.
        addresses: Vec<String>,
        /// Our domains, so they can refetch the set later.
        #[serde(default)]
        dns: Vec<String>,
    },
}

/// The answer to a `Hello`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Welcome {
    /// Accepted. Carries the current address set so the caller can replace what
    /// it holds — this is what makes rotation need no coordination.
    Ok {
        alias: String,
        addresses: Vec<String>,
        #[serde(default)]
        dns: Vec<String>,
    },
    /// Refused, with a reason that is safe to say out loud.
    No { reason: String },
}

/// What may be asked once a connection has been greeted.
///
/// The greeting settles *who*; this is *what*. A connection can carry any number
/// of these, or none — a device that only registers sends none and simply holds
/// the stream open.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "ask", rename_all = "kebab-case")]
pub enum Ask {
    /// Where is the device answering to `name`, and may `caller` reach it?
    ///
    /// Asked peer-to-peer, by the sirji acting for the caller. `caller` is the key
    /// that will actually dial the device, which is what the ticket gets bound to
    /// — usually a device of the asker's, not the asker itself.
    Resolve { name: String, caller: String },

    /// Where is a sibling — another device of the same parent?
    ///
    /// Devices in one constellation have no way to find each other otherwise:
    /// they hold no `network.toml`, and asking a peer would be absurd for
    /// something inside their own fleet. The parent knows all of them, so it
    /// answers, and issues a ticket exactly as it would to an outsider — being
    /// siblings is not a reason to skip authorisation.
    ResolveLocal { name: String },

    /// Resolve `name@alias` on our behalf.
    ///
    /// Asked by one of our own devices, which holds no `network.toml` and so
    /// cannot know who `alias` is. We look the peer up, ask them, and pass the
    /// answer back.
    ResolveFor { name: String, alias: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "say", rename_all = "kebab-case")]
pub enum Say {
    /// The device's id52, and a ticket admitting `caller` to it.
    Resolved {
        device: String,
        ticket: crate::ticket::Ticket,
        /// Where the device can be reached, when we know. Hints, not identity.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        hints: Vec<String>,
    },
    No { reason: String },
}

// ---------------------------------------------------------------------------
// cli <-> daemon
// ---------------------------------------------------------------------------

/// Asked over the unix socket in `$SIRJI_HOME`. Filesystem permission on that
/// socket is the authorization; there are no keys and no policy on this channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
pub enum Request {
    /// Is the daemon up, and what is it listening as?
    Status,
    /// Mint a peer key for someone, record a pending peer, and hand back an
    /// invite they can accept.
    Invite { alias: String },
    /// Complete an invite: mint our identity for them, dial, exchange.
    Accept { alias: String, invite: Invite },
    /// Every relationship, pending or established.
    Peers,
    /// Mint an enrolment identity for a device claiming `name`, and hand back an
    /// invite it can accept. The same two-key invite as pairing.
    DeviceInvite { name: String },
    /// Our own fleet, and which of them are connected right now.
    Devices,
    /// Mint a new handshake key and start listening on it.
    NewAddress { alias: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "kebab-case")]
pub enum Response {
    Status {
        home: String,
        addresses: Vec<AddressInfo>,
        peers: usize,
        pending: usize,
    },
    Invite {
        invite: Invite,
    },
    Accepted {
        alias: String,
    },
    Peers {
        peers: Vec<PeerInfo>,
    },
    Devices {
        devices: Vec<DeviceInfo>,
    },
    NewAddress {
        alias: String,
        key: String,
    },
    Error {
        message: String,
    },
}

/// Everything the other side needs to reach us and recognise us: our addresses,
/// our domains, and the identity we will present to them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invite {
    /// Our handshake keys — where to dial. A peer key cannot be dialled, because
    /// nothing listens on one.
    pub addresses: Vec<String>,
    /// Our domains, so the set can be refetched later.
    #[serde(default)]
    pub dns: Vec<String>,
    /// The peer key we minted for them: how we will be recognised, and their
    /// proof of being the invitee.
    pub identity: String,
    /// Socket addresses we were reachable at when the invite was made.
    ///
    /// **Hints, not identity.** An invite is a point-in-time rendezvous, so
    /// carrying where we were saves a discovery round trip — and works at all in
    /// places where discovery is unavailable. They go stale; the addresses above
    /// are what endure, and the key authenticates the connection either way.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hints: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddressInfo {
    pub alias: String,
    pub key: String,
    pub retired: bool,
    pub bound: bool,
    /// Home relays, and whether we are actually connected to each.
    ///
    /// This is the answer to "am I reachable from outside this network?" — mDNS
    /// covers a LAN, but reaching a peer across the internet needs a relay for
    /// hole-punching and fallback, and discovery to have published where we are.
    #[serde(default)]
    pub relays: Vec<RelayInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayInfo {
    pub url: String,
    pub connected: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub name: String,
    pub keys: Vec<String>,
    /// Awaiting enrolment.
    pub pending: bool,
    /// Holding a connection to us right now. This is the whole liveness story.
    pub live: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    pub alias: String,
    pub peer: Option<String>,
    pub mine: String,
    pub addresses: Vec<String>,
    pub reached_on: Option<String>,
}

impl Invite {
    /// Invites travel by paste, QR and chat message, so they need one line of
    /// text. Base64 of the JSON: opaque enough not to be edited by hand,
    /// transparent enough to debug.
    pub fn encode(&self) -> String {
        use data_encoding::BASE64URL_NOPAD;
        BASE64URL_NOPAD.encode(serde_json::to_string(self).unwrap_or_default().as_bytes())
    }

    pub fn decode(text: &str) -> anyhow::Result<Self> {
        use data_encoding::BASE64URL_NOPAD;
        let bytes = BASE64URL_NOPAD
            .decode(text.trim().as_bytes())
            .map_err(|e| anyhow::anyhow!("not an invite: {e}"))?;
        Ok(serde_json::from_slice(&bytes)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_invite_survives_a_paste() {
        let invite = Invite {
            addresses: vec!["a".into(), "b".into()],
            dns: vec!["example.com".into()],
            identity: "c".into(),
            hints: vec!["127.0.0.1:1234".into()],
        };
        let text = invite.encode();
        assert!(!text.contains('\n'), "must be one line");
        let back = Invite::decode(&format!("  {text}  \n")).unwrap();
        assert_eq!(back.addresses, invite.addresses);
        assert_eq!(back.identity, invite.identity);
        assert_eq!(back.dns, invite.dns);
        assert_eq!(back.hints, invite.hints);
    }

    #[test]
    fn nonsense_is_not_an_invite() {
        assert!(Invite::decode("not an invite").is_err());
    }
}
