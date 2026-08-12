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

/// Our mDNS service name, so sirjis find each other on a LAN without depending on
/// anything outside it.
const MDNS_SERVICE: &str = "sirji";

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
    let builder = Endpoint::builder(presets::N0)
        .secret_key(secret)
        .ca_tls_config(ca_config());
    let builder = with_relays(builder)?;
    builder
        // Every lookup iroh offers, not just one. The N0 preset brings pkarr
        // publish/resolve and DNS, both of which need reachable n0 infrastructure;
        // mDNS needs nothing but the local network, so a LAN — or two sirjis on
        // one machine — works with no infrastructure at all.
        .address_lookup(mdns(true))
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .map_err(|e| anyhow::anyhow!("binding endpoint: {e}"))
}

/// Names a file or directory of extra CA certificates to trust, in PEM form.
///
/// The equivalent of `NODE_EXTRA_CA_CERTS`, `REQUESTS_CA_BUNDLE` or
/// `SSL_CERT_FILE`: every tool that has to survive a corporate network grows one,
/// because the alternative is asking each user to modify a trust store they often
/// do not administer.
pub const EXTRA_CA_ENV: &str = "SIRJI_EXTRA_CA";

/// Overrides which relay servers to use, comma-separated.
///
/// **This is not a tuning knob, it is a requirement.** The relays iroh ships with
/// by default are a handful of hostnames belonging to one organisation, and a
/// corporate web filter blocks a hostname by *category*, wholesale, regardless of
/// certificates. Observed on a Fortinet-filtered network:
/// `aps1-1.relay.n0.iroh.link` returns the firewall's block page while
/// `use1-1.relay.iroh.network` serves the relay untouched — same protocol, same
/// software, different verdict on the domain.
///
/// So a substrate whose premise is having no single point of failure cannot depend
/// on one organisation's hostnames. An app shipping sirji should point this at a
/// relay it runs, ideally on a domain its users already trust, which removes the
/// "approve a new vendor domain" conversation entirely. A relay is small and
/// stateless and forwards bytes it cannot read.
pub const RELAY_ENV: &str = "SIRJI_RELAY";

/// Shared token presented to the relays named by [`RELAY_ENV`].
///
/// A relay with no access control will relay for anyone who finds it, which on a
/// public host means it eventually relays for strangers. A private relay should be
/// configured with `access = { shared_token = [...] }` and its clients given the
/// token here.
///
/// The token authorises *use of the relay*, nothing more. It is not an identity and
/// grants no standing with any sirji: a relay forwards bytes it cannot read.
pub const RELAY_TOKEN_ENV: &str = "SIRJI_RELAY_TOKEN";

/// How TLS certificates are verified when talking to relays and discovery servers.
///
/// **This is the setting that decides whether sirji works inside a company.**
/// Enterprises terminate TLS — Fortinet, Zscaler, Palo Alto, Netskope — so the
/// certificate a relay appears to present is signed by the employer's CA, not by a
/// public one. iroh's default is a copy of Mozilla's roots compiled into the
/// binary, which by construction cannot contain that CA, so the default fails on
/// exactly the networks the product is sold into.
///
/// Two layers, and both are needed in practice:
///
/// 1. **The OS trust store** ([`iroh::tls::CaTlsConfig::system`]). On a managed
///    device the employer's CA is normally installed there, so interception is
///    transparent — this is what a browser does, and it is why a browser works
///    where a tool with bundled roots does not.
/// 2. **Extra roots from [`EXTRA_CA_ENV`]**, for when it is *not* installed —
///    inspection switched on without the CA being pushed, an unmanaged machine, a
///    user without admin rights. Point the variable at the PEM and sirji trusts it
///    without anyone touching the system store.
///
/// What trusting an interceptor actually costs is worth being exact about: it sees
/// relay and discovery **metadata** — which id52 publishes, that two endpoints
/// exchange packets. It cannot read peer traffic. Peer connections authenticate by
/// ed25519 keypair rather than by certificate authority, so there is no CA an
/// interceptor could substitute; a relay forwards bytes it cannot decrypt, and so
/// does the proxy carrying them.
fn ca_config() -> iroh::tls::CaTlsConfig {
    let config = iroh::tls::CaTlsConfig::system();
    match extra_roots() {
        Ok(roots) if !roots.is_empty() => config.with_extra_roots(roots),
        Ok(_) => config,
        Err(e) => {
            // Loud, because silently falling back leaves someone debugging a
            // connection failure whose cause is a typo in a path.
            eprintln!("warning: ignoring {EXTRA_CA_ENV}: {e:#}");
            config
        }
    }
}

/// Read every certificate from the file or directory named by [`EXTRA_CA_ENV`].
fn extra_roots() -> Result<Vec<rustls_pki_types::CertificateDer<'static>>> {
    let Some(path) = std::env::var_os(EXTRA_CA_ENV) else {
        return Ok(Vec::new());
    };
    let path = std::path::PathBuf::from(path);

    let mut files = Vec::new();
    if path.is_dir() {
        for entry in std::fs::read_dir(&path)
            .with_context(|| format!("reading {}", path.display()))?
        {
            let entry = entry?.path();
            if entry.is_file() {
                files.push(entry);
            }
        }
        files.sort();
    } else {
        files.push(path);
    }

    let mut roots = Vec::new();
    for file in files {
        let pem = std::fs::read(&file)
            .with_context(|| format!("reading {}", file.display()))?;
        let mut cursor = std::io::Cursor::new(pem);
        let found: Vec<_> = rustls_pemfile::certs(&mut cursor)
            .collect::<std::result::Result<_, _>>()
            .with_context(|| format!("parsing certificates in {}", file.display()))?;
        if found.is_empty() {
            anyhow::bail!("{} contains no certificates", file.display());
        }
        roots.extend(found);
    }
    Ok(roots)
}

/// Apply [`RELAY_ENV`] if it is set, otherwise leave iroh's defaults alone.
fn with_relays(builder: iroh::endpoint::Builder) -> Result<iroh::endpoint::Builder> {
    let Some(value) = std::env::var_os(RELAY_ENV) else {
        return Ok(builder);
    };
    let value = value.to_string_lossy().to_string();
    let urls: Vec<&str> = value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();

    if urls.is_empty() {
        // An empty value is a deliberate "no relays": direct connectivity only.
        return Ok(builder.relay_mode(iroh::RelayMode::Disabled));
    }
    let map = iroh::RelayMap::try_from_iter(urls.iter().copied())
        .with_context(|| format!("{RELAY_ENV}={value:?} is not a list of relay URLs"))?;
    let map = match std::env::var(RELAY_TOKEN_ENV) {
        Ok(token) if !token.is_empty() => map.with_auth_token(token),
        _ => map,
    };
    Ok(builder.relay_mode(iroh::RelayMode::Custom(map)))
}

/// mDNS lookup. `advertise` says whether to announce ourselves as well as listen:
/// an address should be findable, an identity should not.
fn mdns(advertise: bool) -> iroh_mdns_address_lookup::MdnsAddressLookupBuilder {
    iroh_mdns_address_lookup::MdnsAddressLookup::builder()
        .service_name(MDNS_SERVICE)
        .advertise(advertise)
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
    let builder = Endpoint::builder(presets::N0)
        .secret_key(secret)
        .ca_tls_config(ca_config());
    let builder = with_relays(builder)?;
    builder
        // Resolve, but **do not advertise**. A peer key is an identity, and
        // broadcasting one on the local network would undo the unlinkability the
        // whole design rests on: anyone on the LAN could enumerate every identity
        // we present. Addresses are published; identities never are.
        .address_lookup(mdns(false))
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
