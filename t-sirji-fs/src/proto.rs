//! The t-sirji-fs application protocol.
//!
//! This is **an app's own protocol**, not part of sirji. sirji hands us an
//! authenticated stream and tells us who is on the other end; what we say over it
//! is entirely ours. That boundary is the point of the substrate — the next app
//! defines something completely different over the same wire.

use serde::{Deserialize, Serialize};

/// The first line on any connection: the ticket, when the caller has one.
///
/// A direct dial carries none, which the server may accept or refuse as it likes —
/// sirji authenticates the *key* either way, the ticket is what turns a key into a
/// name.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Knock {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<sirji::Ticket>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum Ask {
    /// What is in this directory, relative to the served root?
    List { path: String },
    /// Send me this file.
    Get { path: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "reply", rename_all = "kebab-case")]
pub enum Say {
    Listing { entries: Vec<Entry> },
    /// The header for a file; `bytes` of content follow on the same stream.
    ///
    /// Length-prefixed rather than delimited, so the reader knows when it is done
    /// without scanning content it has no business interpreting.
    File { name: String, bytes: u64 },
    No { reason: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
    pub bytes: u64,
}
