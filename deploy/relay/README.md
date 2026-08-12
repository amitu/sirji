# Running your own relay

A relay forwards packets between peers that cannot reach each other directly. You
need one when NAT defeats hole-punching, when a network blocks UDP entirely, or —
the reason this directory exists — **when a corporate firewall category-blocks the
default relay hostnames.**

That last case is not hypothetical. Measured on a Fortinet-filtered network:
`aps1-1.relay.n0.iroh.link` returns the firewall's own 403 block page regardless of
what certificates say, while an unrelated host serves fine. Certificates and
category-blocking are different problems and only one of them has a client-side
fix.

**Running the relay on a hostname your users already trust removes the problem
rather than working around it.** There is no new vendor domain to get approved, and
nothing to be classified as unknown.

## What it is not

A relay is **not** a trusted party and **not** part of the identity model. It
forwards bytes it cannot read: peer connections authenticate by ed25519 keypair, so
there is no certificate a relay could substitute and nothing it can decrypt. Losing
a relay costs reachability, never confidentiality — which is why running your own,
or several, is cheap.

## Deploy

```sh
scp -r deploy/relay <server>:/tmp/
ssh <server>
sudo RELAY_HOSTNAME=relay.example.com CONTACT=you@example.com /tmp/relay/install.sh
```

**Point DNS at the box first.** Let's Encrypt validates over HTTP on port 80, so
the A/AAAA record has to exist before the relay starts or issuance fails. The
script checks and refuses rather than leaving you with a relay that serves no
HTTPS.

Then, on a client:

```sh
export SIRJI_RELAY=https://relay.example.com
sirji daemon
sirji status          # the relay line should read 'connected'
```

## Ports

| port | proto | why |
|---|---|---|
| 80 | tcp | ACME HTTP-01 challenge, and the captive-portal probe |
| 443 | tcp | the relay itself — a WebSocket, which is what survives a network that blocks UDP |
| 7842 | udp | QUIC address discovery: tells each peer how it appears from outside, so two peers can try a direct connection instead of paying the relay hop |

If 7842/udp is closed the relay still works and every connection is relayed —
correct, but slower and more expensive than it needs to be.

## Lock it down

The shipped config sets `access = "everyone"`, which is fine for a first
connectivity test and wrong to leave running: anyone who finds the host can relay
through it on your bandwidth. Edit `/etc/iroh-relay/relay.toml`:

```toml
access = { shared_token = ["<a long random string>"] }
```

and give clients the token:

```sh
export SIRJI_RELAY=https://relay.example.com
export SIRJI_RELAY_TOKEN=<the same string>
```

An allowlist of endpoint ids also works, but needs an edit every time a device
appears; a token does not. Either way the token authorises *use of the relay* and
nothing else — it is not an identity and grants no standing with any sirji.

## Checking it

```sh
curl -sS https://relay.example.com/ | head -3     # the relay's page, not a cert error
systemctl status iroh-relay
journalctl -u iroh-relay -f
curl -s localhost:9090/metrics | head             # metrics, loopback only
```

`sirji status` on a client reports the relay and, when it fails, the whole error
chain. Two failures worth recognising:

- **`invalid iroh-relay version header: <empty>`** — reachable, but not an
  iroh-1.0 relay. An old server, or something else answering on that port.
- **an HTML page instead of an error** — a firewall's block page. Certificates are
  irrelevant here; the hostname is being refused by policy.

## More than one

`SIRJI_RELAY` takes a comma-separated list, and clients pick by latency:

```sh
export SIRJI_RELAY=https://relay-eu.example.com,https://relay-us.example.com
```

Setting it empty disables relays entirely — direct connectivity only, useful for
proving that a path really is direct.
