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
sudo RELAY_HOSTNAME=relay.example.com /tmp/relay/install.sh
```

Then, on a client:

```sh
export SIRJI_EXTRA_CA=./relay.crt      # copied from the server
export SIRJI_RELAY=https://relay.example.com
sirji daemon
sirji status          # the relay line should read 'connected'
```

## Certificates — you probably do not want Let's Encrypt

The reflex is ACME. It is usually the wrong call here, and the reason is worth
understanding rather than following.

**The relay never sees your data.** Peers authenticate each other by ed25519
keypair and their session is end to end; the relay forwards bytes it cannot
decrypt. So the certificate is not what stands between an attacker and your
files. It hides *connection metadata* from a passive observer and stops an
impostor posing as the relay — which would buy that impostor traffic analysis and
the ability to drop packets, never the ability to read them.

That makes this an operational choice, so pick the least fragile:

| `CERT_MODE` | when | costs |
|---|---|---|
| `selfsigned` *(default)* | you ship the client | clients set `SIRJI_EXTRA_CA` |
| `manual` | you already have a certificate for the domain | none; reuses your PKI and its renewals |
| `letsencrypt` | strangers with unconfigured clients must connect | port 80, DNS before first start, 90-day renewals, rate limits, an external CA that must stay reachable |

Four new failure modes, for something that is not protecting the payload — that
is the trade `letsencrypt` asks you to make, and it is only worth it when you
genuinely cannot configure the clients.

Two cases where ACME is not merely awkward but unavailable: a relay inside a
corporate network that cannot reach a public CA, and any host without a public
DNS name. Both are ordinary enterprise deployments, and both work fine
self-signed.

Note that `SIRJI_EXTRA_CA` is the same variable used to trust a corporate CA —
one knob doing both jobs, which is a decent sign the seam is in the right place.

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

## What has and has not been verified

Honest about this, because a deployment guide that reads confidently and fails on
contact is worse than one that says where it is unsure:

- **Verified.** Every field name and default here was read out of iroh-relay
  1.0.3's own `Config`, not invented. The file parses as TOML, and `install.sh`
  passes `bash -n`.
- **Not verified.** iroh's deserializer has never seen this file — `Config` is
  private to the relay binary, so it cannot be exercised from outside. The real
  check is running the relay, and that has not been done yet on a real host.

Two mistakes were found by re-checking rather than by running, and both were the
same shape — a file that looked right and meant something else:

- `access` was written after the `[tls]` header, so TOML scoped it into that
  table. iroh would have ignored it and left the relay **open to anyone**.
- `cert_dir` pointed at a subdirectory of the systemd `StateDirectory`, which
  systemd does not create and `ProtectSystem=strict` makes unwritable. Let's
  Encrypt certificates would have failed to cache, re-issuing on every restart
  until the rate limit stopped them.

Expect a third. Read the journal on first start.

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
