//! sirji — a peer-to-peer network substrate.
//!
//! Two kinds of key, and keeping them apart is the whole design:
//!
//! - a **handshake key** is an *address*: what you listen on. A few per sirji,
//!   public, interchangeable, rotatable.
//! - a **peer key** is an *identity*: what you dial *from*. One per relationship,
//!   shown to exactly one peer, and never listened on — which is why per-peer
//!   identity costs nothing and no two peers can correlate you.
//!
//! See `DESIGN.md` for the full model. This crate is what an app embeds to become
//! a sirji device.

pub mod endpoint;
pub mod id52;
pub mod keystore;

pub use endpoint::{ALPN, Connection, Incoming, bind, bind_dialer, dial};
pub use keystore::Keystore;

/// Re-exported so callers need not depend on iroh directly to name a key.
pub use iroh::{Endpoint, PublicKey, SecretKey};
