//! The sealed ticket.
//!
//! When a caller resolves `name@sirji`, that sirji returns the device's id52 and a
//! ticket. The device verifies the ticket and thereby learns who is knocking and
//! as what — **without holding any identity state of its own.** All identity
//! authority stays at the central; a device trusts its owner's signature and
//! nothing else.
//!
//! **Signed, not encrypted.** An earlier draft wanted it encrypted to the device's
//! id52, which is not possible — an id52 is an ed25519 *signing* key and ed25519
//! does no key agreement. It is also unnecessary: encryption would have hidden the
//! ticket from the only party who ever holds it, the caller, and it contains
//! nothing they do not already know. What must be true is that a ticket cannot be
//! **forged** or **lent**. The signature gives the first; binding `caller` gives
//! the second; QUIC already encrypts the wire.

use anyhow::{Result, bail};
use iroh::{PublicKey, SecretKey, Signature};
use serde::{Deserialize, Serialize};

use crate::id52;

/// How long a freshly minted ticket is good for.
///
/// Long enough to dial without a clock-skew argument, short enough that a
/// captured one is not a standing invitation.
pub const LIFETIME_SECS: u64 = 300;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ticket {
    /// The name that was asked for.
    pub name: String,
    /// The key that will present this. Binding it is what stops a ticket being
    /// lent to someone else.
    pub caller: String,
    /// Who the caller is, from the issuer's `network.toml`. Absent for a peer
    /// that arrived through a published address and was never named.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    /// Unix seconds after which this is refused.
    pub valid_until: u64,
    /// The handshake key that signed. A device checks this is genuinely its
    /// parent rather than trusting whatever the caller hands over.
    pub issuer: String,
    /// Signature over the fields above.
    pub signature: String,
}

/// The exact bytes that are signed.
///
/// Length-prefixed rather than concatenated, so no combination of field values
/// can be rearranged into a different ticket with the same signature.
fn signed_bytes(name: &str, caller: &str, alias: Option<&str>, valid_until: u64, issuer: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for field in [name, caller, alias.unwrap_or(""), issuer] {
        out.extend_from_slice(&(field.len() as u32).to_be_bytes());
        out.extend_from_slice(field.as_bytes());
    }
    out.extend_from_slice(&valid_until.to_be_bytes());
    out
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

impl Ticket {
    /// Mint a ticket for `caller` to reach `name`, signed by `issuer`.
    pub fn mint(
        issuer: &SecretKey,
        name: impl Into<String>,
        caller: impl Into<String>,
        alias: Option<String>,
        lifetime_secs: u64,
    ) -> Self {
        let name = name.into();
        let caller = caller.into();
        let issuer_id = id52::encode(&issuer.public());
        let valid_until = now() + lifetime_secs;

        let signature = issuer.sign(&signed_bytes(
            &name,
            &caller,
            alias.as_deref(),
            valid_until,
            &issuer_id,
        ));

        Self {
            name,
            caller,
            alias,
            valid_until,
            issuer: issuer_id,
            signature: data_encoding::BASE64URL_NOPAD.encode(&signature.to_bytes()),
        }
    }

    /// Check a ticket presented by `caller`, on behalf of a device whose parent is
    /// one of `parents`.
    ///
    /// Every condition is checked, and the error says which failed — a device
    /// operator debugging a refusal should not have to guess.
    pub fn verify(&self, caller: &PublicKey, parents: &[String]) -> Result<()> {
        if !parents.contains(&self.issuer) {
            bail!(
                "ticket was signed by {}, which is not our parent",
                self.issuer
            );
        }

        let presented = id52::encode(caller);
        if presented != self.caller {
            // The whole point of binding the caller: a ticket handed to someone
            // else does not work for them.
            bail!(
                "ticket was issued to {}, but {presented} presented it",
                self.caller
            );
        }

        let now = now();
        if now > self.valid_until {
            bail!("ticket expired {} seconds ago", now - self.valid_until);
        }

        let issuer = id52::decode(&self.issuer)?;
        let raw = data_encoding::BASE64URL_NOPAD
            .decode(self.signature.as_bytes())
            .map_err(|e| anyhow::anyhow!("unreadable signature: {e}"))?;
        let raw: [u8; Signature::LENGTH] = raw
            .as_slice()
            .try_into()
            .map_err(|_| anyhow::anyhow!("a signature is {} bytes", Signature::LENGTH))?;

        issuer
            .verify(
                &signed_bytes(
                    &self.name,
                    &self.caller,
                    self.alias.as_deref(),
                    self.valid_until,
                    &self.issuer,
                ),
                &Signature::from_bytes(&raw),
            )
            .map_err(|_| anyhow::anyhow!("the signature does not verify"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (SecretKey, SecretKey, Vec<String>) {
        let parent = SecretKey::generate();
        let caller = SecretKey::generate();
        let parents = vec![id52::encode(&parent.public())];
        (parent, caller, parents)
    }

    #[test]
    fn a_good_ticket_verifies() {
        let (parent, caller, parents) = setup();
        let ticket = Ticket::mint(
            &parent,
            "fs",
            id52::encode(&caller.public()),
            Some("bob".into()),
            LIFETIME_SECS,
        );
        ticket.verify(&caller.public(), &parents).unwrap();
    }

    #[test]
    fn a_ticket_lent_to_someone_else_is_refused() {
        let (parent, caller, parents) = setup();
        let ticket = Ticket::mint(&parent, "fs", id52::encode(&caller.public()), None, LIFETIME_SECS);

        let thief = SecretKey::generate();
        let err = ticket.verify(&thief.public(), &parents).unwrap_err().to_string();
        assert!(err.contains("presented it"), "{err}");
    }

    #[test]
    fn a_ticket_from_a_stranger_is_refused() {
        let (_parent, caller, _parents) = setup();
        let impostor = SecretKey::generate();
        let ticket = Ticket::mint(&impostor, "fs", id52::encode(&caller.public()), None, LIFETIME_SECS);

        // Signed correctly, by the wrong sirji.
        let real_parent = vec![id52::encode(&SecretKey::generate().public())];
        let err = ticket.verify(&caller.public(), &real_parent).unwrap_err().to_string();
        assert!(err.contains("not our parent"), "{err}");
    }

    #[test]
    fn an_expired_ticket_is_refused() {
        let (parent, caller, parents) = setup();
        let ticket = Ticket::mint(&parent, "fs", id52::encode(&caller.public()), None, 0);
        std::thread::sleep(std::time::Duration::from_millis(1100));
        let err = ticket.verify(&caller.public(), &parents).unwrap_err().to_string();
        assert!(err.contains("expired"), "{err}");
    }

    #[test]
    fn tampering_with_any_field_breaks_the_signature() {
        let (parent, caller, parents) = setup();
        let good = Ticket::mint(
            &parent,
            "fs",
            id52::encode(&caller.public()),
            Some("bob".into()),
            LIFETIME_SECS,
        );

        // Escalate the name: the same ticket, aimed at a different device.
        let mut forged = good.clone();
        forged.name = "secrets".into();
        assert!(forged.verify(&caller.public(), &parents).is_err());

        // Promote yourself to a different alias.
        let mut forged = good.clone();
        forged.alias = Some("admin".into());
        assert!(forged.verify(&caller.public(), &parents).is_err());

        // Extend your own validity.
        let mut forged = good.clone();
        forged.valid_until += 86_400;
        assert!(forged.verify(&caller.public(), &parents).is_err());
    }

    #[test]
    fn fields_cannot_be_shuffled_across_the_boundary() {
        // Length-prefixing is what stops ("ab", "c") signing the same bytes as
        // ("a", "bc"), which would let one ticket be reinterpreted as another.
        assert_ne!(
            signed_bytes("ab", "c", None, 1, "x"),
            signed_bytes("a", "bc", None, 1, "x"),
        );
    }
}
