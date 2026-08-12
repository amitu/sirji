# sirji — Implementation plan

The build order from [DESIGN.md](DESIGN.md) § Build order, expanded into
milestones. Each milestone ends in **something you can run**; none ends in a layer
of types waiting for a caller.

One question is settled by a measurement before the code that depends on it — see
§ Spikes. Everything else is decided.

---

## How we work

- **Line by line, no hurry.** Every line close to its final form. Where we
  knowingly write something temporary, it is called out loudly rather than left
  to be discovered.
- **End-to-end testable, not unit testable.** A milestone is done when a command
  produces the right observable behaviour. Pure logic (encoding, config parsing,
  name lookup) is tested directly against in-memory literals; everything else is
  tested by running it.
- **No anticipatory types.** A type appears when a caller needs it. If it cannot
  be reviewed in its usage context, it is too early.
- **All dependencies in the workspace manifest.** Each crate declares
  `<name>.workspace = true`. Never `cargo add`; the exact lines go in by hand so
  versions cannot drift between crates.

---

## Spikes

One measurement, taken before the milestone that depends on it. A second question
is recorded here as already resolved, since the reasoning is worth keeping.

### Spike A — what does an endpoint cost, and can they share network state?

**Why this is now a much easier question.** An earlier draft made every identity a
listening endpoint — several hundred sockets, hole-punchers, relay connections and
republished discovery records on one host, all rediscovering the same network
behind the same NAT. That is gone: DESIGN.md listens only on **handshake keys**
(a handful per sirji), while per-relationship **peer keys are dialled from and
never listened on.**

So the question is no longer "can iroh survive 500 endpoints" but **"what does an
outbound-only identity cost?"** — because that is the one that scales with
relationship count.

**The spike.** Bind 1, 5, 20, 100 endpoints in one process, with and without a
relay. Measure: file descriptors, RSS, relay connections established, idle CPU and
network, and time to bind. Then two questions that decide how far the privacy dial
can be turned:

1. **What does an endpoint used only for dialling cost?** It needs no published
   discovery record and no inbound reachability; whether iroh still holds a home
   relay and republishes for it is the question. **This is the one that matters** —
   it is multiplied by relationship count.
2. **Can endpoints share a socket, netcheck, or relay connection?** Relays route by
   node id, so one relay connection serving several identities is plausible in
   principle. If iroh exposes it, the cost of peer keys collapses regardless.
3. **Can a dial from key K be made without binding a long-lived endpoint at all** —
   bind, dial, keep the connection, drop the endpoint's discovery machinery? An
   established QUIC connection does not need its originator to stay discoverable.

**What the answer changes.** Nothing about the design — address and identity are
separate whatever the numbers say. It sets **documented guidance**: how many
relationships a sirji carries comfortably, and whether peer-key endpoints should be
bound lazily on first dial rather than at startup. Record the measurement in
DESIGN.md § Identity.

### Spike B — resolved: the ticket is signed, not encrypted

DESIGN.md once required the sealed ticket **encrypted to the device's id52**. That
is not possible: an id52 is an ed25519 *signing* key and ed25519 does no key
agreement. The options were a second X25519 key per device, an Edwards→Montgomery
conversion, or dropping the encryption.

**Decided: drop it.** The encryption would have hidden the ticket from the only
party who ever holds it — the caller — and it contains nothing they do not know.
What must be true is that a ticket cannot be forged or lent, so:

```
ticket = { name, caller, alias, valid_until }  +  central's signature
```

The device verifies the signature and checks the connecting key equals `caller`.
QUIC already encrypts the wire. No X25519, no key conversion, no second key type.
Recorded in DESIGN.md § Connection flow; nothing left to spike.

---

## Workspace layout

```
sirji/
  Cargo.toml         workspace manifest; ALL deps in [workspace.dependencies]
  rust-toolchain.toml
  sirji/             the library. This is what an app embeds.
  sirji-cli/         the `sirji` binary: operator commands, and the harness
                     every milestone is demonstrated through.
```

Two crates, not one: the library must stay usable by an app that has its own
binary, and keeping the CLI separate is what stops operator conveniences leaking
into the embedding API.

There is no `sirji-core`/`sirji-proto` split. Without a second implementation in
this workspace there is nothing to share, and inventing the seam now would be an
anticipatory type at crate scale.

## Dependency policy

Added at the milestone that first needs one, never before:

| milestone | dependency | for |
|---|---|---|
| M1 | `iroh`, `tokio`, `anyhow` | the wire; async; error plumbing at the edges |
| M2 | `data-encoding`, `rand` | id52 text form; key generation |
| M3 | `serde`, `toml` | `network.toml` |
| M4 | `bincode` | length-delimited structs on a stream |
| M5 | `ed25519-dalek` (or iroh's re-export) | signing and verifying tickets |
| M6 | `clap` | once the CLI has more than a handful of commands |
| M7 | `hickory-resolver` | `_sirji.<domain>` TXT lookup |

`thiserror` arrives when the library has an error type worth matching on — that
is, when an app must distinguish "unknown name" from "no live holder". Not before.

---

## Milestones

### M1 — the wire

Two processes, two endpoints, dial by node id, echo a byte over an ALPN.

**Also the point at which iroh's actual current API is read and pinned** — the
version, endpoint construction, ALPN registration, accept loop shape, relay
configuration and discovery. Nothing downstream should be written against a
guessed signature.

**Done when:** two terminals, one prints an id52-shaped address, the other dials
it and both see the byte. Works between two machines, not just two processes on
one host.

**Not yet:** identity meaning, config, names, tickets.

### M2 — id52 and the keystore

- **id52.** 32-byte ed25519 public key ⇄ 52-character text. 32 bytes is 51.2
  base32 characters, hence 52 unpadded. **Alphabet: base32hex, lowercase, no
  padding** — the DNSSEC alphabet, which is what "dnssec-style" in DESIGN.md
  means; it is domain-safe and sorts in the same order as the bytes. Fixed now
  because it appears in DNS records and config files, where changing it later is
  a migration.
- **The keystore.** `~/.sirji/keys/<id52>.private-key`, 32 bytes, mode 0600.
  Written before the `network.toml` entry that references it, per DESIGN.md.
- **Self-verification.** On load, derive the public key from the secret and check
  it matches the filename. A mismatch is an error at startup, not at first dial.

Conversion to and from iroh's own key types lives here and nowhere else, so
iroh's string forms never leak into our files.

**Done when:** `sirji key new` mints a key and prints its id52; `sirji key ls`
lists the store and verifies every entry; a deliberately corrupted file is
reported clearly.

### M3 — `network.toml`

Parse all three sections; resolve a name to the device keys allowed to answer to
it; recognise a `[[peer]]` with `mine` and no `peer` as a pending invite.

Pure and side-effect-free: it takes text and answers questions. Tested against
literals in memory, including the cases that should fail — a name no `[[device]]`
declares, a key claiming a name it is not listed under, a duplicate alias.

**Done when:** `sirji net check` validates a file and prints its interpretation —
peers, invites pending, devices and the names they may answer to, handshake keys
and where each is published.

### M4 — registration, heartbeat, roster

The first thing with two roles talking to each other.

- Device connects to its central and claims a name; central accepts only if a
  `[[device]]` entry declares that name and lists the key.
- Heartbeat every 30s carrying an **opaque application payload** that sirji stores
  and returns but never interprets.
- Central keeps the live roster; three consecutive misses (90s) drops an entry.
- The library exposes the roster to the owning app.

**Done when:** `sirji serve` in one terminal, `sirji device chat` in two others,
and `sirji roster` shows both holders of `chat` with their last-seen times. Kill
one; it disappears within 90s. Restart it; it returns.

**Not yet:** resolution by callers, tickets.

### M5 — the ticket

`{ name, caller, alias, valid_until }` signed by central. Mint at central, verify
at the device, and refuse a dial that presents no valid ticket — the refusal path
is the part worth testing, since it is the only enforcement the substrate has.

**Done when:** a device rejects a direct dial with no ticket, rejects one signed by
the wrong central, rejects one whose `caller` is not the connecting key, rejects an
expired one, and accepts a good one.

### M6 — `name@sirji`, end to end

Three processes: a central, a device, a caller. Resolve → receive device id52 plus
ticket → dial the device directly → present the ticket → the app gets a stream
with the caller's alias attached. Plus the caching rule: reuse a resolution until
a dial fails, then re-resolve. No TTL.

**Done when:** `sirji dial chat@<alias>` opens an authenticated stream and the
device prints who connected and as what. Killing and moving the device causes one
failed dial, a silent re-resolution, and a working connection.

This is the milestone an app can be written against. The embedding API is whatever
the CLI needed to get here, and no more.

### M7 — bootstrap

The ways a relationship begins and is kept reachable, in the order they are
useful:

1. **The handshake-key exchange** — mint a peer key, dial their address, both sides
   write their `[[peer]]`. The listener distinguishes first contact from an
   existing relationship purely by whether the dialling key is known.
2. **The two-key invite** — an address plus the peer key minted for them, which
   doubles as proof they are the invitee.
3. **DNS** — `_sirji.<domain>` TXT lookup returning one or more handshake keys,
   recorded on the `[[peer]]` so the address can be refetched later.
4. **Introduction** — a handshake key passed on with a vouch.
5. **Rotation and draining** — publish a new address, keep the old one bound,
   hand the current one to peers that arrive on the retired key, and unbind only
   once no `[[peer]]` names it in `reached_on`.

**Done when:** two sirjis on two machines that have never met complete a
handshake, write mirror-image `[[peer]]` entries, and reach each other's named
services by alias. Then rotate one side's address and watch the other pick it up
on its next dial, with `sirji key retire` refusing until it has.

---

## Deliberately not built

Named so their absence reads as a decision rather than an omission: all of
Layer-2 (sync, durable ordered storage, offline delivery, replication, conflict
resolution, file transfer); the social/chat application; ticket expiry and
revocation beyond `valid_until`; horizontal scaling of a central; multi-hop
introduction chains and trust assertions; per-requester device addresses.

## Risks, most dangerous first

1. **Relay dependence.** Everything works on one LAN long before it works between
   two NATs, and the whole reachability story rests on hole-punching and relays.
   M1 must be tested across two networks, or M6 will pass locally and fail in
   reality.
2. **A peer key reused across two relationships.** This is the one failure that
   silently destroys the central property, and it is invisible in testing because
   everything still works. Every mint must assert the key is new, and `sirji net
   check` must fail loudly if any `mine` appears in two `[[peer]]` entries.
3. **Retiring an address too early.** Unbinding a handshake key while a `[[peer]]`
   still names it in `reached_on` strands that peer — recoverable only via `dns`,
   and not at all without it. `sirji key retire` must refuse while any peer still
   points at it, rather than warning.
4. **Endpoint cost setting an undocumented ceiling** (Spike A). Not a design risk
   any more, but users need a stated number.
4. **iroh API drift.** Pin the version at M1 and record it; do not track a moving
   target while the substrate is being written.
