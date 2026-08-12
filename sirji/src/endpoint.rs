//! Binding and dialling.
//!
//! An endpoint is bound with a specific secret key, because in iroh the key *is*
//! the address: `Endpoint::id()` is the public half of the key it was bound with.
//! That is the fact the whole identity model rests on — a key you can be reached
//! at costs a bound endpoint, and a key you only dial *from* does not.
//!
//! It is also why a listener never needs to ask who is calling: iroh mutually
//! authenticates before any application byte moves, so `Connection::remote_id()`
//! is already the dialer's key. Known key means an existing relationship; unknown
//! means a handshake.

use anyhow::{Context, Result};
use iroh::{Endpoint, PublicKey, SecretKey, endpoint::presets};

/// Re-exported so an embedding app never has to name `iroh` itself. The crate
/// boundary is deliberate: iroh's types and text forms stay on this side of it.
pub use iroh::endpoint::{Connection, Incoming};

/// Every sirji connection negotiates this ALPN. Apps layer their own protocol on
/// top of the stream; they do not get their own ALPN, so a peer needs one
/// connection to us, not one per app.
pub const ALPN: &[u8] = b"/sirji/1";

/// Bind an endpoint that listens as `secret`'s public key.
///
/// Use this for handshake keys — the addresses we publish. It performs discovery
/// and holds a relay connection, which is what makes us reachable and what makes
/// a listening key more expensive than a dialling one.
pub async fn bind(secret: SecretKey) -> Result<Endpoint> {
    Endpoint::builder(presets::N0)
        .secret_key(secret)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .map_err(|e| anyhow::anyhow!("binding endpoint: {e}"))
}

/// Bind an endpoint used only to dial, as `secret`'s public key.
///
/// Use this for peer keys — our identity toward one relationship. It advertises
/// no ALPN, so nothing can connect *to* it; it exists to put our identity on the
/// outbound connection.
///
/// How much cheaper this is than [`bind`] is the open question in `PLAN.md`
/// § Spike A, and it is the cost that scales with relationship count.
pub async fn bind_dialer(secret: SecretKey) -> Result<Endpoint> {
    Endpoint::builder(presets::N0)
        .secret_key(secret)
        .bind()
        .await
        .map_err(|e| anyhow::anyhow!("binding dialer endpoint: {e}"))
}

/// Dial `address` from `endpoint`, giving us a stream pair.
///
/// `address` is a handshake key — a peer key is never dialled, because nothing
/// listens on one.
pub async fn dial(endpoint: &Endpoint, address: PublicKey) -> Result<Connection> {
    endpoint
        .connect(address, ALPN)
        .await
        .with_context(|| format!("dialling {}", crate::id52::encode(&address)))
}

/// Dial `address` at a known socket address, skipping discovery.
///
/// Discovery maps a key to where it currently is; when that infrastructure is
/// unavailable — or when proving the transport itself — the address can be
/// supplied directly. The connection is authenticated by the key either way:
/// supplying a socket says *where*, never *who*.
pub async fn dial_at(
    endpoint: &Endpoint,
    address: PublicKey,
    socket: std::net::SocketAddr,
) -> Result<Connection> {
    let addr = iroh::EndpointAddr::new(address).with_ip_addr(socket);
    endpoint
        .connect(addr, ALPN)
        .await
        .with_context(|| format!("dialling {} at {socket}", crate::id52::encode(&address)))
}
