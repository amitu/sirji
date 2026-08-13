# Using a relay

A relay forwards packets between peers that cannot reach each other directly —
NAT that will not hole-punch, or a network that blocks UDP outright. sirji is the
**client** side of that relationship: it decides which relay to use and how to
trust it. Running the relay is iroh's business, and this document does not
duplicate it.

> **Deploying one:** follow iroh's own instructions for `iroh-relay`
> (<https://iroh.computer>, and `cargo install iroh-relay --features server`).
> They maintain it; a copy here would drift from theirs and be wrong in a way
> that is hard to notice.

## Why you will need your own

The relays iroh ships with by default are a handful of hostnames belonging to one
company. Measured on a Fortinet-filtered corporate network, with the intercepting
CA trusted so certificates were not the variable:

| host | result |
|---|---|
| `use1-1.relay.n0.iroh.link` | 403, firewall block page |
| `usw1-1.relay.n0.iroh.link` | 403, firewall block page |
| `euc1-1.relay.n0.iroh.link` | 403, firewall block page |
| `aps1-1.relay.n0.iroh.link` | 403, firewall block page |
| both staging relays | 403, firewall block page |
| `dns.iroh.link/pkarr` (discovery) | 403, firewall block page |

**One firewall rule against one domain removes every default relay and the
discovery server at once**, and there is no third-party public iroh relay to fall
back on — Tailscale's DERP servers speak a different protocol. For a substrate
whose premise is having no single point of failure, the default transport is one.

The same measurement had better news in it: **UDP left that network freely** and
plain DNS resolved. Nothing was blocked for being peer-to-peer; everything was
blocked for being on `iroh.link`. So only *coordination* needs to move to a
hostname the network does not refuse — once peers find each other they can go
direct, and the relay carries nothing.

A relay on a hostname your users already trust removes the problem rather than
working around it: no new vendor domain to get approved, and nothing to be
classified as unknown.

## Pointing sirji at it

Configuration comes from `$SIRJI_HOME/config.toml`, and any of it can be
overridden by an environment variable for a single run.

```toml
# $SIRJI_HOME/config.toml

# Relays to use instead of the defaults. An empty list means direct only.
relay = ["https://relay.example.com"]

# Presented to those relays when they are access-controlled.
# relay_token = "a-long-random-string"

# Extra CA certificates to trust, as a PEM file or a directory of them. Needed
# for a relay with a private certificate, and for a corporate network that
# terminates TLS with a CA that is not in the system store.
# extra_ca = "/etc/sirji/relay.crt"
```

| variable | overrides |
|---|---|
| `SIRJI_RELAY` | `relay`, comma-separated. Empty disables relays entirely |
| `SIRJI_RELAY_TOKEN` | `relay_token` |
| `SIRJI_EXTRA_CA` | `extra_ca` |

Check it took:

```sh
sirji status
```

```
address  default    current  bound      9epsg8svd3bnr84q9j3ql8a3unh8nvc4cs0e6s896nejvb7o7ps0
  relay  connected  https://relay.example.com/
```

## Certificates

**Do not reach for Let's Encrypt.** The relay never sees your data — peers
authenticate each other by ed25519 keypair and their session is end to end, so
the relay forwards bytes it cannot decrypt. The certificate hides *connection
metadata* from a passive observer and stops an impostor posing as the relay,
which would buy that impostor traffic analysis and packet dropping, never the
ability to read anything.

So this is an operational choice, and ACME's costs — port 80 exposed, DNS before
first start, 90-day renewals, rate limits, an external CA that must stay reachable
— buy nothing that matters here. Two ordinary cases make it impossible anyway: a
relay inside a network that cannot reach a public CA, and a host with no public
DNS name.

Give the relay a certificate you already have, or a self-signed one, and point
clients at it with `extra_ca`. That is the same setting used to trust a corporate
CA — one knob, both jobs.

## Reading the failures

`sirji status` prints the whole error chain, not just its outermost sentence.
Three worth recognising:

- **`invalid iroh-relay version header: <empty>`** — reachable, but not an
  iroh-1.0 relay. An older server, or something else answering on that port.
- **an HTML page instead of an error** — a firewall's block page. Certificates
  are irrelevant; the hostname is being refused by policy.
- **`invalid peer certificate: UnknownIssuer`** — TLS is being terminated by
  something whose CA you do not trust. Either the relay's own certificate is
  private (set `extra_ca`) or the network is intercepting (set `extra_ca` to the
  employer's CA, or get it installed in the system trust store).
