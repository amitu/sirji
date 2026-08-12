//! id52 — the text form of a public key.
//!
//! An ed25519 public key is 32 bytes, which is 51.2 base32 characters, so 52
//! unpadded. The alphabet is **base32hex, lowercase**: the DNSSEC alphabet, which
//! is domain-safe and sorts in the same order as the underlying bytes.
//!
//! iroh renders keys as 64 hex characters. That form never appears in our files
//! or on our wire — this module is the only place the two meet.

use anyhow::{Result, bail};
use data_encoding::BASE32HEX_NOPAD;
use iroh::PublicKey;

/// Every id52 is exactly this long. 32 bytes, base32, unpadded.
pub const LEN: usize = 52;

/// Render a public key as its id52.
pub fn encode(key: &PublicKey) -> String {
    BASE32HEX_NOPAD.encode(key.as_bytes()).to_lowercase()
}

/// Parse an id52 back into a public key.
pub fn decode(s: &str) -> Result<PublicKey> {
    if s.len() != LEN {
        bail!("an id52 is {LEN} characters, got {}: {s:?}", s.len());
    }
    // The canonical form is lowercase; accept uppercase rather than rejecting a
    // key someone shouted at us, since base32hex has no case ambiguity.
    let bytes = BASE32HEX_NOPAD
        .decode(s.to_uppercase().as_bytes())
        .map_err(|e| anyhow::anyhow!("not a valid id52: {e}"))?;
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("an id52 must decode to 32 bytes"))?;
    Ok(PublicKey::from_bytes(&bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use iroh::SecretKey;

    #[test]
    fn round_trips_and_is_always_52_lowercase() {
        for _ in 0..64 {
            let key = SecretKey::generate().public();
            let text = encode(&key);

            assert_eq!(text.len(), LEN, "{text}");
            assert_eq!(text, text.to_lowercase(), "{text}");
            assert_eq!(decode(&text).unwrap(), key);
        }
    }

    #[test]
    fn accepts_uppercase_but_emits_lowercase() {
        let key = SecretKey::generate().public();
        let text = encode(&key);
        assert_eq!(decode(&text.to_uppercase()).unwrap(), key);
    }

    #[test]
    fn rejects_the_wrong_shape() {
        let key = SecretKey::generate().public();
        let good = encode(&key);

        assert!(decode("").is_err(), "empty");
        assert!(decode(&good[..LEN - 1]).is_err(), "too short");
        assert!(decode(&format!("{good}0")).is_err(), "too long");
        // 'w' is outside the base32hex alphabet (0-9, a-v).
        assert!(decode(&format!("w{}", &good[1..])).is_err(), "bad alphabet");
    }

    #[test]
    fn is_not_iroh_hex() {
        // Guards the boundary: if iroh's Display ever became base32 we would want
        // to notice rather than silently agree.
        let key = SecretKey::generate().public();
        assert_ne!(encode(&key), key.to_string());
        assert_eq!(key.to_string().len(), 64);
    }
}
