//! `network.toml` — the known net.
//!
//! Public halves only. Nothing here is secret, which is what makes the file safe
//! to keep in version control.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::id52;

pub const FILE: &str = "network.toml";

/// The whole file.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Network {
    /// Our domains. Every current handshake key is published at each of them, and
    /// the list is shared with peers so they can refetch our addresses later.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dns: Vec<String>,

    /// What we listen on. All functionally identical.
    #[serde(rename = "handshake-key", default, skip_serializing_if = "Vec::is_empty")]
    pub handshake_keys: Vec<HandshakeKey>,

    /// One entry per relationship.
    #[serde(rename = "peer", default, skip_serializing_if = "Vec::is_empty")]
    pub peers: Vec<Peer>,

    /// Our own fleet: a name, and the keys allowed to answer to it.
    #[serde(rename = "device", default, skip_serializing_if = "Vec::is_empty")]
    pub devices: Vec<Device>,
}

/// An address: a key we listen on and hand out.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandshakeKey {
    /// So it can be talked about and rotated.
    pub alias: String,
    /// The id52.
    pub key: String,
    /// No longer advertised, but still bound until every peer has moved off it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub retired: bool,
}

/// A relationship: a pair of keys, plus where to reach them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Peer {
    /// What we call them. Local to us; their name for us may differ.
    pub alias: String,
    /// Their peer key — how we recognise them when they dial.
    /// Absent while an invite is outstanding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer: Option<String>,
    /// Our peer key for them — what we dial from. Shown to exactly one peer.
    pub mine: String,
    /// Their handshake keys: everywhere we may dial them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub addresses: Vec<String>,
    /// Their domains, if any, for refetching the set later.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dns: Vec<String>,
    /// Which of *our* handshake keys they last arrived on. This is what makes
    /// retiring an address decidable rather than a guess.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reached_on: Option<String>,
}

/// One of our own devices: a name, and the keys authorised to answer to it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Device {
    /// What it answers to. Peers address it as `name@us`.
    pub name: String,
    /// The device's own keys. What it presents when it dials us, and what peers
    /// dial after resolving the name. Empty while enrolment is outstanding.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<String>,
    /// An identity minted for one device and sent to it, which it presents to
    /// prove it is the invitee — the same two-key invite as pairing. Cleared once
    /// enrolled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invite: Option<String>,
}

impl Device {
    /// Invited but not yet arrived.
    pub fn is_pending(&self) -> bool {
        self.keys.is_empty()
    }
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Peer {
    /// An invite that has been sent and not yet accepted.
    pub fn is_pending(&self) -> bool {
        self.peer.is_none()
    }
}

impl Network {
    pub fn path_in(home: &Path) -> PathBuf {
        home.join(FILE)
    }

    pub fn load(home: &Path) -> Result<Self> {
        let path = Self::path_in(home);
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    /// Write the file, replacing it atomically so a crash cannot leave a
    /// half-written known net.
    pub fn save(&self, home: &Path) -> Result<()> {
        let path = Self::path_in(home);
        let tmp = path.with_extension("toml.tmp");
        let text = toml::to_string_pretty(self).context("serialising network.toml")?;
        std::fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &path).with_context(|| format!("replacing {}", path.display()))?;
        Ok(())
    }

    /// The addresses we currently advertise: every handshake key not retired.
    pub fn current_addresses(&self) -> Vec<String> {
        self.handshake_keys
            .iter()
            .filter(|k| !k.retired)
            .map(|k| k.key.clone())
            .collect()
    }

    pub fn handshake_key(&self, key: &str) -> Option<&HandshakeKey> {
        self.handshake_keys.iter().find(|k| k.key == key)
    }

    pub fn handshake_key_by_alias(&self, alias: &str) -> Option<&HandshakeKey> {
        self.handshake_keys.iter().find(|k| k.alias == alias)
    }

    /// Look a dialling key up. This is the known/unknown split: `Some` is an
    /// existing relationship, `None` means a handshake.
    pub fn peer_by_key(&self, key: &str) -> Option<&Peer> {
        self.peers.iter().find(|p| p.peer.as_deref() == Some(key))
    }

    pub fn peer_by_alias(&self, alias: &str) -> Option<&Peer> {
        self.peers.iter().find(|p| p.alias == alias)
    }

    /// A peer we minted `mine` for and are still waiting on. Looked up by the key
    /// we sent them, which is the invite's proof of identity.
    pub fn pending_by_mine(&self, mine: &str) -> Option<&Peer> {
        self.peers
            .iter()
            .find(|p| p.is_pending() && p.mine == mine)
    }

    pub fn device_by_name(&self, name: &str) -> Option<&Device> {
        self.devices.iter().find(|d| d.name == name)
    }

    /// A device we minted an enrolment identity for and are still waiting on.
    pub fn pending_device_by_invite(&self, invite: &str) -> Option<&Device> {
        self.devices
            .iter()
            .find(|d| d.is_pending() && d.invite.as_deref() == Some(invite))
    }

    /// Which of our devices holds this key, if any. This is the device-side
    /// equivalent of the known/unknown split on peer connections.
    pub fn device_by_key(&self, key: &str) -> Option<&Device> {
        self.devices
            .iter()
            .find(|d| d.keys.iter().any(|k| k == key))
    }

    /// The names a device key may answer to.
    pub fn names_for_device_key(&self, key: &str) -> Vec<&str> {
        self.devices
            .iter()
            .filter(|d| d.keys.iter().any(|k| k == key))
            .map(|d| d.name.as_str())
            .collect()
    }

    /// Reject anything that would quietly break an invariant. Called on load by
    /// the daemon and by `sirji net check`, because both of these are silent
    /// failures that testing would never surface.
    pub fn check(&self) -> Result<()> {
        for key in &self.handshake_keys {
            id52::decode(&key.key)
                .with_context(|| format!("handshake key {:?}", key.alias))?;
        }

        let mut seen_alias = std::collections::HashSet::new();
        let mut seen_mine = std::collections::HashMap::new();
        for peer in &self.peers {
            if !seen_alias.insert(&peer.alias) {
                bail!("two peers share the alias {:?}", peer.alias);
            }
            id52::decode(&peer.mine)
                .with_context(|| format!("peer {:?}: `mine`", peer.alias))?;
            if let Some(other) = seen_mine.insert(&peer.mine, &peer.alias) {
                // The one failure that destroys unlinkability while everything
                // still appears to work.
                bail!(
                    "peers {:?} and {:?} share the identity key {} — a peer key must \
                     never be shown to two peers",
                    other,
                    peer.alias,
                    peer.mine
                );
            }
            if let Some(their) = &peer.peer {
                id52::decode(their)
                    .with_context(|| format!("peer {:?}: `peer`", peer.alias))?;
            }
            for address in &peer.addresses {
                id52::decode(address)
                    .with_context(|| format!("peer {:?}: address", peer.alias))?;
            }
            if let Some(on) = &peer.reached_on
                && self.handshake_key_by_alias(on).is_none()
            {
                bail!(
                    "peer {:?} was last reached on {on:?}, which is not one of our \
                     handshake keys",
                    peer.alias
                );
            }
        }

        let mut seen_name = std::collections::HashSet::new();
        for device in &self.devices {
            if !seen_name.insert(&device.name) {
                bail!("two devices share the name {:?}", device.name);
            }
            for key in &device.keys {
                id52::decode(key)
                    .with_context(|| format!("device {:?}", device.name))?;
            }
            if let Some(invite) = &device.invite {
                id52::decode(invite)
                    .with_context(|| format!("device {:?}: invite", device.name))?;
            }
        }
        Ok(())
    }

    /// Handshake keys that can be unbound: retired, and named by no peer.
    pub fn drained(&self) -> Vec<&HandshakeKey> {
        self.handshake_keys
            .iter()
            .filter(|k| k.retired)
            .filter(|k| {
                !self
                    .peers
                    .iter()
                    .any(|p| p.reached_on.as_deref() == Some(k.alias.as_str()))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SecretKey;

    fn key() -> String {
        id52::encode(&SecretKey::generate().public())
    }

    fn parse(text: &str) -> Network {
        toml::from_str(text).unwrap()
    }

    #[test]
    fn round_trips_through_toml() {
        let mut net = Network {
            dns: vec!["example.com".into()],
            ..Default::default()
        };
        net.handshake_keys.push(HandshakeKey {
            alias: "public".into(),
            key: key(),
            retired: false,
        });
        net.peers.push(Peer {
            alias: "kiran".into(),
            peer: Some(key()),
            mine: key(),
            addresses: vec![key()],
            dns: vec!["kiran.example".into()],
            reached_on: Some("public".into()),
        });
        net.devices.push(Device {
            name: "fs".into(),
            keys: vec![key()],
            invite: None,
        });

        let back: Network = toml::from_str(&toml::to_string_pretty(&net).unwrap()).unwrap();
        back.check().unwrap();
        assert_eq!(back.peers[0].alias, "kiran");
        assert_eq!(back.devices[0].name, "fs");
        assert_eq!(back.dns, vec!["example.com".to_string()]);
    }

    #[test]
    fn an_empty_file_is_a_valid_empty_net() {
        let net = parse("");
        net.check().unwrap();
        assert!(net.current_addresses().is_empty());
    }

    #[test]
    fn a_reused_identity_key_is_rejected() {
        let shared = key();
        let net = Network {
            peers: vec![
                Peer {
                    alias: "a".into(),
                    peer: Some(key()),
                    mine: shared.clone(),
                    addresses: vec![],
                    dns: vec![],
                    reached_on: None,
                },
                Peer {
                    alias: "b".into(),
                    peer: Some(key()),
                    mine: shared,
                    addresses: vec![],
                    dns: vec![],
                    reached_on: None,
                },
            ],
            ..Default::default()
        };
        let err = net.check().unwrap_err().to_string();
        assert!(err.contains("never be shown to two peers"), "{err}");
    }

    #[test]
    fn a_duplicate_alias_is_rejected() {
        let net = Network {
            peers: vec![
                Peer {
                    alias: "same".into(),
                    peer: None,
                    mine: key(),
                    addresses: vec![],
                    dns: vec![],
                    reached_on: None,
                },
                Peer {
                    alias: "same".into(),
                    peer: None,
                    mine: key(),
                    addresses: vec![],
                    dns: vec![],
                    reached_on: None,
                },
            ],
            ..Default::default()
        };
        assert!(net.check().unwrap_err().to_string().contains("share the alias"));
    }

    #[test]
    fn reached_on_must_name_a_key_we_have() {
        let net = Network {
            peers: vec![Peer {
                alias: "a".into(),
                peer: Some(key()),
                mine: key(),
                addresses: vec![],
                dns: vec![],
                reached_on: Some("gone".into()),
            }],
            ..Default::default()
        };
        assert!(net.check().unwrap_err().to_string().contains("not one of our"));
    }

    #[test]
    fn retired_keys_drain_only_when_no_peer_names_them() {
        let mut net = Network {
            handshake_keys: vec![
                HandshakeKey { alias: "v1".into(), key: key(), retired: true },
                HandshakeKey { alias: "v2".into(), key: key(), retired: false },
            ],
            peers: vec![Peer {
                alias: "a".into(),
                peer: Some(key()),
                mine: key(),
                addresses: vec![],
                dns: vec![],
                reached_on: Some("v1".into()),
            }],
            ..Default::default()
        };
        assert!(net.drained().is_empty(), "v1 is still in use");

        net.peers[0].reached_on = Some("v2".into());
        let drained: Vec<_> = net.drained().iter().map(|k| k.alias.clone()).collect();
        assert_eq!(drained, vec!["v1".to_string()]);
    }

    #[test]
    fn current_addresses_exclude_retired() {
        let net = Network {
            handshake_keys: vec![
                HandshakeKey { alias: "v1".into(), key: key(), retired: true },
                HandshakeKey { alias: "v2".into(), key: key(), retired: false },
            ],
            ..Default::default()
        };
        assert_eq!(net.current_addresses(), vec![net.handshake_keys[1].key.clone()]);
    }

    #[test]
    fn a_dialling_key_is_known_or_it_is_a_handshake() {
        let theirs = key();
        let net = Network {
            peers: vec![Peer {
                alias: "known".into(),
                peer: Some(theirs.clone()),
                mine: key(),
                addresses: vec![],
                dns: vec![],
                reached_on: None,
            }],
            ..Default::default()
        };
        assert_eq!(net.peer_by_key(&theirs).unwrap().alias, "known");
        assert!(net.peer_by_key(&key()).is_none());
    }
}
