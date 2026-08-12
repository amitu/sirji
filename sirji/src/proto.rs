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
