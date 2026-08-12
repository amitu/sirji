# sirji — Design

The decided design of sirji's network and relationship layer, settled over design
sessions in June 2026 and reconciled in August 2026. This is the substrate
everything stands on — sirji the app, and every other app built on it. Product
features (the social/chat surface) remain deferred; the protocol no longer is.

**The worked example throughout** is `builder@acme`: a build-and-test allocator
run by an organisation, which hands compute to callers who ask for it. It is the
shape of the substrate's first real consumer, and it exercises every part of
Layer-1 — many machines answering one name, short-lived connections, and an app
that must decide who may ask for what.

This document is **fork-free by rule**: where two designs were possible, one is
chosen and recorded. No options, no "either/or". Anything genuinely unsettled is
named in § Deferred and nowhere else.

Rationale and posture live in [overview.md](overview.md). Vision beyond v1 is
[road-ahead.md](road-ahead.md) and is not scope.

---

## Scope — Layer-1

sirji v1 is **Layer-1**: identity, relationships, and the establishment of
authenticated, enriched connections. It ends the moment an app holds an
authenticated stream to the right device. Apps define their own protocol above
that line.

**Layer-2 — sync, durable ordered storage, offline delivery, replication,
conflict resolution, file transfer — is not in v1** and is not designed. What
Layer-1 owes it, as a contract, so Layer-2 can land without a wire change:

- **Relationship identity is stable across reconnects.** The handshake key a peer
  holds for you does not change because a dial failed or a device moved. Layer-2
  may key durable storage on it.
- **Enrichment is available at connect time** — the peer's alias, when we have
  one — before any application byte is read.
- **Reachability is observable.** The live roster (§ Registration and liveness)
  tells an app when a device became reachable or stopped answering, which is the
  signal a queue flush needs.

Layer-2 may **not** assume ordering, delivery, durability, or retry from
Layer-1. A sirji stream is an authenticated pipe and nothing more.

---

## Identity — id52

Every actor holds an ed25519 keypair. The public key is encoded as **id52**: a
domain-safe, 52-character representation (dnssec-style base32). Knowing a peer's
id52 *is* authenticating it — the connection is mutually authenticated by the
keypairs themselves; there is no separate auth layer, no tokens, no certs, no VPC
assumptions.

### Handshake keys are the identity

A **handshake key** is a key you hand out and then listen on. It is how peers
reach you and how you dial them; there is no second, "real" identity behind it.
You mint as many as you like and give each one to whomever you choose:

```
public      published in DNS         → anyone may knock
family      handed out in person     → ten relatives know it
lee         minted for one person    → exactly one peer knows it
```

**A handshake key's distribution set is its correlation set.** Everyone you gave
the same key to can determine they share a contact — but they could work that out
anyway, because you handed each of them the same string. Nothing leaks that
handing out the key did not already leak.

That single fact is what makes the privacy property both honest and adjustable:

| you mint | they can correlate | cost |
|---|---|---|
| one key for everyone | everyone, with each other | 1 endpoint |
| one key per circle | within a circle, not across | a handful of endpoints |
| one key per peer | nobody | one endpoint per relationship |

So **unlinkability is a dial, not an absolute** — set per relationship, by how
narrowly you distribute the key. One key per peer gives exactly the pairwise
identity an earlier draft made mandatory; it is now the strongest setting of a
control rather than a rule everyone pays for.

**Both directions are symmetric.** A relationship is a *pair of handshake keys* —
the one you gave them, and the one they gave you. Each side chose its own
granularity independently, so you can hold a one-peer key for someone who reaches
you through their public one.

### Why not one key per relationship, always

An earlier draft required it: *"no keypair is ever shown to two peers"*, without
exception. It was dropped because it is unaffordable, and pretending otherwise
would have failed at the first real deployment.

In iroh, one endpoint is one ed25519 secret key is one address. A key you can be
reached at must be a *bound endpoint* — its own socket, its own hole-punching
state, its own relay connection, its own republished discovery record. A person
with several hundred relationships would hold several hundred of those, and every
one of them would rediscover the same network conditions behind the same NAT at
the same public address. **Dialling out has the same shape**: you connect *from*
an endpoint, so N outbound identities means N endpoints too.

It also bought less than it appeared to. The observer that a shared transport
identity exposes you to is a relay — and every endpoint on one host shares one
public IP, so a relay correlates them by address regardless. The expensive
resource was being spent to close a channel that stays open anyway.

What genuinely needed protecting is **two peers comparing notes**, and the
distribution-set rule gives that at whatever granularity you are willing to pay
for.

Rejected outright, and recorded so it is not re-proposed: **a rotating pool of
transport keys** — M keys serving N relationships. It gives only partition-level
unlinkability (everyone assigned the same key still correlates), and costs
assignment, a rotation schedule, peer notification, and an offline-during-rotation
case. Worse privacy than one-key-per-circle, for considerably more machinery.

### The one asymmetry: callers are always unlinkable

A caller dials *from* their own key, so QUIC's mutual authentication already tells
the receiver which key is knocking. Two consequences:

- **No application-level identity handshake is needed at all.** Central takes the
  connecting key, looks it up in `[[peer]]`, and has the alias. The transport did
  the work.
- **The caller's exposure is set by their own granularity, not yours.** Somebody
  who dials your public key from a one-peer key of their own remains unlinkable,
  whatever you can see. Each side controls its own side of the edge.

### Device id52s

Devices sit inside a constellation and are addressed differently: an internal
address, like an internal IP. Mapped to names, never published, handed out only
inside a resolution. **One per device.**

This was once recorded as a reluctant deviation — the earlier draft wanted a
per-requester device address so the one-key-per-peer rule held here too. It is no
longer an exception to anything: it is the same reasoning as above, applied where
the peer count is highest and most transient. Two callers who both resolve
`builder@acme` can compare device id52s and learn they reached the same box, which
inside one owner's constellation is not a threat.

The upgrade path, if it ever becomes one: the **address pool**. A device pre-mints
a batch of keypairs, registers only their public id52s with central, and central
issues one per requester, replenishing at heartbeat — central never holds a device
private key. This is the prekey-bundle pattern, and it changes registration and
resolution only, not the ticket or the wire.

---

## The entity model

- A **sirji** is a fixed node — an always-on server, a person's stable presence
  on the network. People run one each; so do groups and orgs.
- A **device** is anything holding its own keypair and attached to a sirji: a
  phone, a laptop, **an app**. Your sirji and your devices form your
  constellation; sirji↔sirji connects people to each other.
- **Apps are devices.** The substrate ships as the **sirji crate** (Rust). An app
  embeds the crate and thereby *is* a sirji device — its own keypair, its own
  id52, movable to another machine by moving the key. There is no
  app-registers-with-local-daemon, no forwarding hop, no local agent to keep
  alive. The allocator on some box is a device of the `acme` sirji exactly as a
  phone is a device of a personal one.
- A **group sirji** is a fixed node **owned by another sirji**. Any sirji can
  declare itself a fixed node owned by some other; the **owner authors its
  `network.toml`**. That is governance without key-sharing: the group's keys live
  on the group's node, authority over its membership lives with the owner, and
  many humans stand behind one service because the owner's pen says so — each
  member holding their own handshake key for it.

### The sirji is also the default host

A sirji is a doorman, but it is not *only* a doorman: it is the one node in a
constellation that is always on, so it is also where durable services belong.
Because Layer-1 has no queue (§ Scope), a service reachable only when a laptop
happens to be awake will silently drop what is sent to it. So:

- Services that must not miss anything — chat, a public place, anything with
  correspondents — run as devices **on the sirji node itself**.
- Phones, laptops and short-lived workers are **edge** devices: they initiate,
  they answer while present, and nothing is lost when they sleep.

An allocator's workers are legitimately edge devices — a reservation is negotiated
while both sides are live, and a missed dial is simply a worker not chosen. That is
why an allocator needs no part of Layer-2, and why a chat app will.

---

## network.toml — sirji's known net

One file per sirji, authored by its owner. It is the only store of
**cryptographic** identity in the system — every id52 sirji knows, and nothing
else. Hand-editable, `git`-trackable, and read **deterministically**.

**It is static configuration, not prose for a model.** No LLM ever reads it, and
none ever should. It records cryptographic facts — whose key is whose, which key
may answer to which name — and a cryptographic fact is either true or it is not.
There is nothing here to weigh.

A relationship is between two people, not between two apps. You and a peer shake
hands once; every app either of you ever runs consumes that handshake through
enrichment. **Apps never perform handshakes and never keep cryptographic
identity stores of their own.** This kills the wall every prior p2p attempt hit:
one relationship per app.

### Two files, two jobs — and `network.md` is not this file

`network.toml` is **sirji's known net**. It is deliberately thin: handshake keys,
the relationships they carry, our own devices, and name→device bindings. No
groups, no tags, no visibility rules. It does **not** model an organisation, and it must not
try.

Real orgs need groups of groups, ad-hoc groups, roles, rotations, cost centres —
structure that is genuinely hard to pin down in static schema and that every org
shapes differently. That belongs one layer up, in an **app-level `network.md`**:
a semantic, LLM-inferrable model of the org, authored by the org, in whatever
semantics the org invents. An app's `policy.md` refers to it rather than restating
it, and apps are expected to follow the same pattern. See
[patterns/network-md.md](patterns/network-md.md).

The split is not stylistic. It follows from what each file is load-bearing for:

| | `network.toml` | `network.md` |
|---|---|---|
| owns | id52s, devices, handshake keys, name bindings | the org's shape and vocabulary |
| read by | the sirji crate, deterministically | an LLM, semantically |
| gates | nothing — it *identifies*. Possession of a handshake key is the only gate | **policy** — already an LLM call, so rich semantics are free |
| lives with | the sirji | the app |
| when wrong | a peer is misidentified, or a name answered by the wrong device | a verdict is wrongly reasoned |

**Security is deterministic; policy is semantic.** You cannot spend a model call
deciding whether to accept a connection — that is a token per dial and a
non-deterministic security boundary. But an app like the allocator is *already*
making a model call to reach its verdict, so arbitrarily rich org semantics cost
nothing extra there.

**The alias is the join key.** sirji resolves an incoming id52 to an **alias** and
ships that in the sealed ticket. The app looks the alias up
in its own `network.md` to learn everything sirji does not know — where this
person sits in the org, what they are on the hook for this week, which ad-hoc
group they joined. Neither file needs to know the other's internals; they meet
at a name.

(Consequence for an app that already had its own identity table: `policy.md` sheds
any id52→username layer — that is `network.toml`'s job. `policy.md` keeps rules
written against names that arrive already resolved, and refers to `network.md` for
what those names mean organisationally.)

### Grammar

```toml
# network.toml — acme

# ── handshake keys: what we listen on and dial from. ───────────────────
#    each key's distribution set is its correlation set.
[[handshake-key]]
alias     = "public"
key       = "kh3m9x2q..."
published = "example.com"    # in DNS: anyone may knock

[[handshake-key]]
alias = "family"             # not published; handed out in person
key   = "kh8w4nf5..."

[[handshake-key]]
alias = "for-lee"            # minted for one person: nobody can correlate it
key   = "k2te7bd9..."

# ── peers: one entry per relationship. a pair of handshake keys. ───────
[[peer]]
alias = "dana"
peer  = "k51qzi5uqu5dijh7at4a9y2gk8pd0m3bqrxvce6nfu1s2h4j"  # the key they gave us
mine  = "family"                                            # the key they know us by

[[peer]]
alias = "kiran"
peer  = "k9m2ha4t..."
mine  = "public"

# pending: `for-lee` minted and sent, not yet answered.
[[peer]]
alias = "lee"
mine  = "for-lee"

# ── our own devices: a name, and the keys allowed to answer to it ──────
[[device]]
name = "phone"
keys = ["k4pv8ac1..."]

[[device]]
name = "chat"
keys = ["k6hw3lfd...", "k1jd7so2..."]   # two machines answer to `chat`
```

**`[[handshake-key]]`** is a key we hand out and listen on: an alias so it can be
talked about and rotated, the key itself, and optionally where it is published.
Several are normal and expected, and **how narrowly you distribute each one is the
whole privacy control** (§ *Handshake keys are the identity*). A key given to one
person is what an earlier draft called a per-relationship identity.

**`[[peer]]`** records a relationship, and a relationship is a **pair of handshake
keys**: `peer` is the key they gave us — their id52, which we dial and which
authenticates them — and `mine` names which of *our* handshake keys they know us
by. Only public halves appear here; no secret is ever in this file, which is
git-trackable by design.

`mine` is an **alias, not an id52.** The key itself is in the
`[[handshake-key]]` entry with that alias, so it is written once and referenced by
name — which is also what makes "which peers would a rotation disturb?" a lookup
rather than a search.

An entry with `mine` but no `peer` is **pending**: we have minted a key and sent it
out, and they have not answered yet. An invite is exactly this — a handshake key
whose distribution set is one person — so it is a state of an ordinary record, not
a separate mechanism.

*(This section was called `[[handshake]]` in the June design. Renamed because
`[[handshake]]` and `[[handshake-key]]` sitting adjacent in one file is a
readability trap — they are entirely different things. The relationships are peers;
the keys are handshake keys.)*

**`[[device]]`** is our own fleet: a device name and the keys authorised to answer
to it. It exists because a person's own devices are **not** peers, so `[[peer]]`
cannot describe them. **This is also the whole name-binding rule** — listing two
keys under one name is how a name is load-balanced across machines, and a key not
listed here can claim nothing.

### What this file does *not* contain

No groups. No tags. No `visible_to`. No per-name visibility of any kind. Each was
considered and cut, and the reasoning is worth keeping because it is the same
reasoning three times over.

**No groups.** A group is not a cryptographic fact — it is a name someone made
up — so it has no business in the store of cryptographic facts. All grouping and
aliasing lives in the app's `network.md`, where a model reads it and naming
semantics can be as rich as the organisation actually is.

**No tags.** Their only consumer was `visible_to`. With that gone they had no job
here, and they were a poorer version of what `network.md` does properly.

**No `visible_to`.** Minting the sealed ticket *is* the authorization act, so
central necessarily makes one decision per resolution — but that decision does
not need a policy language, because the app is going to check anyway. The ticket
carries the alias; the device opens it and decides. A substrate check could only
ever be the *weaker* of the two, since resolution happens before any app is
reached and therefore can only narrow, never widen. Two enforcement points where
one suffices.

**No `open` / `public` flag.** This is the one that looked unavoidable — central
must somehow decide whether to serve a peer it has never met. It dissolves once
you see that **a handshake key's distribution is its policy**: publish the key in DNS
and anyone may knock; hand it to five friends and exactly those five can. The
capability *is* the key. A flag would encode the same fact a second time, in a
place where it could disagree with the first.

**No `[names]` section.** It bound a name to the devices allowed to answer to it —
which is exactly what `[[device]]`'s own `name` and `keys` already say. It was the
same fact written twice, in two places that could disagree. Gone.

**And the `PrincipalSet` abstraction goes with them.** It existed to serve
`claimable_by` and `visible_to` through one resolver. With both gone there is
nothing to resolve: a name lookup is a scan of `[[device]]` for a matching `name`.
It was scaffolding for mechanisms that no longer exist.

### The substrate has no access gate — key possession is the gate

What is left is a single sentence: **you can only reach what you were given a key
to knock on.** Everything past that is the app's business.

Concretely, central's rule when a peer resolves a name is only *does this name
exist, and is a live holder available* — because by the time anyone can ask, they
are already a known peer. Becoming known required knocking on a handshake key, and
knocking required being given its key.

This is why the substrate needs no policy language. It is not that authorization
was skipped; it is that **the authorization already happened, out of band, when
you decided who to give a key to.** Every subsequent question — may this person
run a test, post in this place, read this file — is downstream of a model call or
an app rule, and belongs there.

A name no `[[device]]` entry declares is claimable by nobody, so nothing answers
to it.

---

## Keys on disk

`network.toml` holds public halves only. The secrets live outside it, and this
section says where.

**This is a local storage decision, invisible to the wire.** Nothing here is
protocol: two sirjis that store keys differently interoperate perfectly, so it can
change later without a protocol change.

### One file per key

```
~/.sirji/
  network.toml                        public only; safe to commit
  .gitignore                          contains `keys/`
  keys/
    k77bqxr2m9d4pv8ac1ye5tgz0nkj….private-key    32 bytes, mode 0600
    kh3m9x2q….private-key
```

The filename **is** the index: given `mine = "k77bqxr…"` in `network.toml`, the
secret is at `keys/k77bqxr….private-key`. No counter, no derivation, no lookup
table. A new relationship is 32 random bytes written to one new file; forgetting a
relationship is deleting that file and its `[[peer]]` entry.

Because the filename carries the public half, **the store is self-verifying**:
derive the public key from the secret and check it matches the name. A corrupted or
misfiled key is caught immediately rather than at the next dial.

### Why not one derived master seed

An earlier draft of this section derived every key from a single 32-byte seed via
HKDF, on the argument that the whole recovery kit would then be one static secret
plus a git-tracked text file. **That was the wrong call, and it is recorded here so
it is not proposed again.**

- **Blast radius.** A leaked key file compromises one relationship. A leaked seed
  compromises **every relationship, past and future** — and there is no rotation:
  you cannot change a root that every existing edge depends on without
  re-handshaking every peer individually. The argument that "the seed and the
  keystore sit in the same directory with the same permissions" conflates *full
  filesystem compromise* with *partial disclosure*, and partial disclosure is the
  common case: a stray backup, a mis-synced folder, a debugger dump, a screenshot.
- **It contradicts the model.** Identity here is *independent keys*. One
  file per key makes the storage layout match that; a shared root makes every edge
  structurally dependent on one value for no benefit the model asked for.
- **It invented a correctness hazard.** Derivation needs a monotonic index that must
  never be reused, because reuse silently hands one key to two peers and breaks the
  one-key-per-peer guarantee. That hazard existed *only* because of derivation. One file per
  key deletes the entire class of bug, along with the counter and the KDF.
- **It forecloses hardware keys.** A derived seed must be readable by software. Per-key
  storage can later become per-key handles into an OS keychain, a TPM, or a phone's
  secure enclave, where the secret is never exportable at all.
- **The backup advantage was overstated.** Both designs come down to "back up
  `~/.sirji/`", an operation every backup tool already handles. Derivation's one
  genuine edge is that a *stale* backup stays complete — but `network.toml` also
  changes with every new relationship, so a stale backup loses the new aliases and
  peer ids regardless. It bought less than it appeared to.

### Backup, stated plainly

`keys/` must be backed up, and it changes as relationships form. **Losing it is
unrecoverable** — there is no directory to look people up in again (§ *There is no
directory*), so each lost relationship can only be rebuilt by reaching that person
through some other channel and handshaking afresh.

One consolation of independent keys: recovery degrades gracefully. A partial
restore keeps exactly the relationships whose keys survived, rather than being
all-or-nothing.

### Two ordering rules

**Write the key file before the `network.toml` entry.** If a crash lands between
the two, an orphaned key file is harmless garbage that can be collected later,
whereas a `[[peer]]` entry whose key is missing is a relationship that can never be
used again. Order accordingly — including for invites, where the key exists well
before the peer answers.

**Device keys stay on their device.** Each device holds its own key under its own
`keys/`, generated at first registration and never shared upward. A stolen phone
yields one device key, not a constellation.

---

## Registration, names, and liveness

On startup a device dials its central sirji and **claims a name**: `builder`,
`phone`, `watcher`. Names are the stable addressing layer; device id52s hang
off them.

- **Name claims are gated by `network.toml`** — a device may claim a name only if
  a `[[device]]` entry declares that name and lists its key. Ungated registration would be
  impersonation; this single rule prevents it. First registration is the pairing
  ceremony; afterwards, key continuity.
- **Multiple devices may claim the same name.** That is load balancing: central
  picks among the live holders of a name. No separate load-balancer concept
  exists.

**Liveness is sirji's job, not the app's.** Because central picks among *live*
holders, the substrate must know who is live:

- After registering, a device sends a **heartbeat every 30 seconds**.
- Central keeps a **live roster**: name → the set of live device id52s, each with
  its `last_seen`.
- **Three consecutive misses (90s) drops a device from the roster.** It rejoins
  by registering again.
- A heartbeat carries an **opaque application payload** — bytes sirji stores and
  hands back with the roster, never interprets. This is how the allocator ships
  capacity and load through one mechanism without sirji learning anything about
  test machines.
- The crate **exposes the roster to the owning app**, so an app that needs to
  choose among its own devices (the allocator's controller picking workers) reads
  it rather than building a parallel presence system.

---

## Connection flow — control plane and data plane

The central sirji is DNS-plus-doorman, never a proxy:

1. An inbound connection arrives at the central sirji **asking for a name**
   ("builder").
2. Central checks only that the name exists and that a holder is live. There is no
   visibility rule to consult — see § *What this file does not contain*.
3. If yes, central returns the **device id52** behind the name — picking a live
   holder from the roster — **and a sealed ticket**.
4. The peer dials the device **directly** — p2p, mutually authenticated — and
   presents the ticket. Central carries lookups, never traffic.

**The ticket — enrichment travels with the connector.** Alongside the device
id52, central hands the peer a sealed payload:

- **encrypted to the device's id52** — only that device can open it;
- **signed by the central sirji's private key** — the device verifies it came
  from its owning sirji, full stop;
- **containing the identity context**: the name being addressed, and the peer's
  alias from `network.toml` — or **no alias at all**, if they arrived through a
  published handshake key and have never been named.

The device decrypts, verifies the signature, and knows exactly who is knocking
and as what — **devices hold no `network.toml` and no identity state at all**. All
identity authority concentrates at the central; a device trusts its owner's
signature and nothing else. A connection arriving without a valid ticket is
refused without further thought — the device *cannot* know who it is, by
construction.

**Caching:** a resolved device id52 is held — ticket alongside — and reused until
the device stops answering, then re-resolved at central. No TTL machinery in v1;
re-resolve-on-failure is the whole rule. This is what keeps central lookups-only
and makes future id52 rotation safe: rotation just looks like a failed dial
followed by a fresh resolution.

**Enrichment:** an app sees a peer as **an alias** — carried in the ticket, never
raw identity ceremony, never a synced store. A peer without a ticket is simply
unknown: rejected by construction, the cheapest enforcement there is. An
*unnamed* peer is a legitimate case rather than an error — it is what arriving
through a published handshake key looks like, and the app decides what such a peer may
do.

**Scoping:** an app is scoped by *which sirji it is a device of*. Attach the
allocator to a sirji that knows only the relevant network, not to a sensitive
root. An app can only ever name what its sirji's network contains.

---

## Bootstrap — how anyone comes by an id52 at all

**No identity is ever discovered. Only a handshake key is** — and a handshake key
is the identity, so discovery and introduction are the same act. There is nothing
behind the key to find.

Sorted by discoverability:

| | discoverable? | how you come by it |
|---|---|---|
| **handshake key** | **yes — the only kind that is** | DNS, a QR code, a link, or handed over by a peer |
| **device id52** | never | resolved from central per dial, by a peer that already has a relationship |

### The handshake-key exchange

```
Alice knows only H_B, a handshake key of Bob's.
Alice picks which of her own keys Bob should know her by — H_A.
(If she wants to be uncorrelatable with Bob's other contacts, she mints
 a fresh key for him alone. Otherwise she reuses an existing one.)

  Alice                                              Bob
    │  ──── dial H_B, authenticated as H_A ────────►   │
    │                                                 │  sees a stranger: H_A
    │                                                 │  decides whether to accept
    │  ◄─── accepted ─────────────────────────────────  │
    │                                                 │
    │  records peer=H_B, mine="<alias of H_A>"        │  records peer=H_A,
    │                                                 │          mine="<alias of H_B>"
    │  ═══ all later traffic: dial H_B from H_A ═══════╡
    │      the same two keys, for as long as they know each other
```

**One round trip, and nothing is minted during it.** The keys already exist —
choosing which one to present *is* the privacy decision, and it was made before
dialling. There is no re-key step, no second address to learn, and no window in
which the two sides disagree about which key is current.

QUIC's mutual authentication does the rest: Bob's side sees `H_A` on the
connection, so once he accepts, looking `H_A` up in `[[peer]]` gives him Alice's
alias on every subsequent dial without any identity exchange at all.

Note the consequence for `network.toml`: its `[[peer]]` entries are the *residue*
of handshakes that happened, not something you populate by looking people up.

### A handshake key's distribution is its policy

There is **one kind of handshake key** and no flags on it. What varies is who you
gave it to:

- **published** in `_sirji.<domain>` — anyone may knock. The deliberate public
  face; linkage accepted exactly as far as the publication reaches.
- **private** — shared with friends, a team, a mailing list. Only they can knock,
  because only they have it.
- **single-recipient** — minted for one person. Nobody can correlate it, because
  nobody else has it. This is the strongest privacy setting and it is the same
  mechanism, not a different one.

That is the entire policy, and it needs no mechanism: **the capability is the
key.** A flag saying "this one is public" would encode a second time what the act
of publishing already established, in a place where the two could disagree.

**Rotation is the one thing this costs.** When a handshake key was only used for
first contact, retiring it was free — existing relationships lived on separate
pairwise keys. Now it *is* the address, so retiring one strips every peer that
knows it, and a private key has no discovery path they could rediscover you
through. See § Deferred.

```
_sirji.example.com.  TXT  "id52=k51qzi5uqu5dijh7at4a9y2gk8pd0m3bqrxvce6nfu1s2h4j"
_sirji.example.com.  TXT  "id52=k8w2nf5r..."      # a second handshake key; knock on either
```

The underscore-prefixed subdomain is the standard convention for
service-specific records (`_dmarc`, `_acme-challenge`) and works in every
registrar's zone editor today. Earlier drafts wrote this as a `SIRJI` record type
and as a `<name>.<domain>.sirji` lookup; **neither is deployable** — a new RRTYPE
needs IANA registration, and `.sirji` is not a TLD. This form needs nothing but a
TXT record.

### The per-peer invite

An invite is **not a separate mechanism** — it is a handshake key whose
distribution set is one person, sent before they have answered:

1. Mint a key, give it an alias (`for-lee`), and record a `[[peer]]` entry with
   `alias = "lee"`, `mine = "for-lee"` and no `peer` yet. Optionally pre-place
   `lee` in an app's `network.md`.
2. Send that key to them out of band — a link, a QR at a desk, a message on some
   other network.
3. They dial it from a key of their own choosing.
4. Our side sees a connection arriving at `for-lee`, a key sent to nobody else —
   **so the dialer is Lee by construction.** Fill in `peer` with the key they
   presented. The relationship is complete.

**Nobody approves anything**, because the approval already happened: we decided
who Lee was when we minted a key for her and chose where to send it. And because
her alias could be pre-placed in `network.md`, she has whatever app access it
implies from the instant she accepts.

Since the key was only ever sent to one person, this is also the maximum-privacy
case: Lee cannot be correlated with any other contact, and we keep listening on
that key for her indefinitely.

### Introduction

- **By introduction** — a mutual peer passes along a handshake key, and vouches.
  Mechanically this is just the handshake-key exchange with a hint about who is
  knocking; the vouch is what makes accepting reasonable rather than blind.

### There is no directory, and that is the point

If you have neither a handshake key nor a mutual peer, **you cannot reach someone at
all.** No user search, no enumerable namespace, nothing to scrape. Mass
unsolicited contact is structurally expensive because there is nothing to
enumerate — which is precisely what email and phone numbers, with their global
namespaces, cannot say.

It is also the hardest problem in the system, and the one that killed earlier p2p
attempts: a new person has zero peers and nothing to type. The three moves above
are the whole answer, so the app layer has to make them effortless — a QR scanned
in the room, a link sent over some other network, an introduction that is one tap
for the voucher.

  ```
  _sirji.example.com.  TXT  "id52=k51qzi5uqu5dijh7at4a9y2gk8pd0m3bqrxvce6nfu1s2h4j"
  _sirji.example.com.  TXT  "id52=k8w2nf5r..."      # a second handshake key; knock on either
  ```

  The underscore-prefixed subdomain is the standard convention for
  service-specific records (`_dmarc`, `_acme-challenge`) and works in every
  registrar's zone editor today. Earlier drafts wrote this as a `SIRJI` record
  type and as a `<name>.<domain>.sirji` lookup; **neither is deployable** — a new
  RRTYPE needs IANA registration, and `.sirji` is not a TLD. This form needs
  nothing but a TXT record.

**Scaling:** vertical first — central is control-plane only, so a good machine
goes far; resolutions cache, so load is lookups, not traffic. Horizontal scaling
is deferred and acknowledged unsolved; the TXT-list of multiple handshake keys is
the seam it will use, and `network.toml` being plain text means replication can
start life as `git pull`.

---

## Addressing

Services are addressed **`name@sirji`** — `watcher@acme`. This is the **one
canonical spelling**; the crate resolves it to a device id52, caches it, and dials.
(An earlier design note also used a dotted FQDN form, `builder.acme`. Same idea,
second spelling — dropped. One spelling only.)

**The two halves resolve in different places, and that is the whole trick:**

- **Right of `@` — *who*. Resolved locally, by the caller.** Either an **alias**
  in our own `network.toml`, in which case we already hold a handshake key for
  that sirji and dial it directly; or a **DNS name** (`chat@example.com`), in which
  case we look up `_sirji.example.com` and knock on a handshake key, which
  establishes the relationship before anything else happens.
- **Left of `@` — *what*. Resolved remotely, by the callee's sirji**, which
  looks the name up in its own `network.toml`, picks a live holder from the
  roster, and returns a device id52 plus a sealed ticket.

Neither side ever consults a global directory. There isn't one.

**Aliases are local — this is a petname system.** `dana` in our `network.toml`
is unrelated to `dana` in anyone else's, so `chat@dana` is meaningful only from
our own machine. That is the intended resolution of Zooko's triangle — names are
memorable and secure rather than global — and it has a real consequence for the
app layer: **an address cannot be shared as text.** "Send me your handle" does
not work here. Introducing two people means passing a handshake key, so the app
must make introduction as cheap as sending a link.

A watcher service is, for all intents, a sirji-backed service of a person *or a
group* (a shared service several people stand behind), long-lived within the
relationship that knows it; an app's policy speaks `name@sirji` and never sees the
plumbing.

---

## The wire

iroh underneath: QUIC connections, mutual authentication by keypair, relays and
holepunching so NAT and location never matter. The sirji crate wraps it; an app
defines its own protocol on top of authenticated, enriched connections. Remote
machines run the same substrate — operational access is an app protocol under
grants, never ssh.

---

## Verification — epistemological graph traversal

With per-relationship keys there is no global key to compare. *"Is the entity behind
this key the one behind that key?"* is answerable only by walking trust
assertions through the relationship graph, peer by peer — which is how
knowing-someone actually works among humans. Two strangers connect by one of
exactly two moves:

- **Publication** — one of them has deliberately published a handshake key
  (domain TXT, link, QR) and accepts the linkage that implies: everyone who holds
  that key can correlate each other.
- **Introduction** — a mutual peer vouches and passes a handshake key, brokering
  a fresh handshake. An introduction chain is the unit of traversal; the general
  machinery grows later, but the single introduction is in from the start —
  without it, strangers could only ever meet through publication.

---

## The app layer (bounded sketch)

Recorded to show the substrate suffices — the product spec stays deferred:

- sirji the app is the chat / group / public-places mix (WhatsApp + Reddit
  registers), with a **markdown-based virtual UI**: apps ship limited interactive
  surfaces as templates (rq's `view.md` rendering is the prior art).
- An assistant of yours may answer your chats on your behalf. Per-peer escalation
  rules govern it — *"any question from this person: show me the full transcript
  before responding."* A response is either given now, gated on the human, or
  given now and revised later. Which of those modes are legal is the app
  protocol's contract — a chat may gate on a human; an allocator never does.

**Public places: the addressing works; the state does not.** Worth separating,
because the two halves have very different difficulty.

*Addressed* — a public place is a **group sirji** (a node owned by some person's
sirji, who authors its `network.toml`) running a place service under a name, with
a **published handshake key**. A stranger with no relationship knocks on that
handshake key published at `_sirji.<domain>`, dialling from a key of its own
choosing, and arrives with no alias at all. Anonymous participation
falls out for free: join with a fresh key and that persona is unlinkable to any
other, which is exactly the anonymity this design set out to provide.
Nothing further is needed from Layer-1.

*Unsolved* — **shared mutable state**: the ordering of a thread many peers write
to, who stores its history, who serves it to a newcomer, who moderates, and who
pays for the storage and bandwidth. None of that is an addressing question and
none of it is answered here. It is Layer-2 plus app design, and it is where the
app work will start.

---

## Build order

Minimal and end-to-end at every step; each step is something that runs.

1. **Crate skeleton + iroh hello-world.** Two binaries, two endpoints, dial by
   id52, echo a byte. Proves the wire.
2. **Keystore + id52.** Generate a keypair, write `keys/<id52>.private-key`, read
   it back by id52, encode/decode ed25519 ↔ id52, verify the filename matches the
   secret.
3. **`network.toml` parser.** All three sections; name→device lookup; a `[[peer]]`
   without `peer` recognised as an invite. Pure, testable against a literal in
   memory.
4. **Register + heartbeat + live roster.** A central that accepts a name claim
   only from a key `[[device]]` lists, tracks liveness, and can list who holds a
   name.
5. **The sealed ticket.** Mint, encrypt-to-device, sign-by-central, verify at
   device; a device refuses an un-ticketed dial.
6. **`name@sirji` end to end.** Resolve → receive id52 + ticket → dial device
   directly → present ticket → app gets an enriched stream. Three processes: a
   central, a device, a caller.
7. **Bootstrap.** The handshake-key exchange (choose a key, dial, both record the
   edge); the single-recipient invite; `_sirji.<domain>` TXT lookup; single-hop
   introduction.

Then the first app consumes the crate: a worker's registration becomes step 4, its
controller's worker-pick becomes the roster read, and caller → worker becomes
step 6.

**Not built in v1:** anything in § Deferred, and all of Layer-2.

---

## Roads not taken

An earlier design of this substrate was authority-free and peer-symmetric. It is
recorded here as rejected, so it does not drift back in:

- **Two locked ALPNs** for subordinate↔parent and peer↔peer traffic — dropped in
  favour of apps defining their own protocol above an authenticated stream.
- **Scheduled id52 rotation with peer acknowledgement** — dropped in favour of
  re-resolve-on-failure, which achieves the same thing with no schedule, no
  acknowledgement, and no state: a rotated address is indistinguishable from a
  device that moved.
- **`<id52>://path` as a URI scheme** — dropped in favour of `name@sirji`.
- **A kernel-enforced isolation rule** separating always-on nodes from
  subordinate ones — dropped in favour of "apps are devices", plus scoping by
  which sirji a device belongs to.

- **A rotating pool of transport keys** — M keys serving N relationships, reassigned
  on a schedule. It gives only partition-level unlinkability (peers sharing a key
  still correlate) while costing assignment, rotation, peer notification and an
  offline-during-rotation case. One key per circle is better privacy for less
  machinery.
- **A mandatory per-relationship key** — *"no keypair is ever shown to two peers"*,
  without exception. Unaffordable: a reachable key is a bound endpoint, and several
  hundred relationships means several hundred sockets, hole-punchers, relay
  connections and republished discovery records, all rediscovering one network
  behind one NAT at one address. Kept as the *strongest setting* of a control
  instead of a rule everyone pays for (§ Identity).

The load-bearing difference is **where authority lives**: there, nowhere; here,
concentrated at a constellation's central sirji. That concentration is exactly what
makes the sealed ticket and the no-identity-state-on-devices rule possible.

That earlier design also carried a **file-sync engine** — subscriptions,
parent-tracked push, conflicts surfaced as events, per-device capacity envelopes.
That is not rejected, only deferred: it is the starting material for Layer-2.

---

## Deferred

Named here and nowhere else. Each is unsettled, not undecided-between-options.

- **All of Layer-2** — sync, durable ordered storage, offline delivery,
  replication, conflict resolution, file transfer.
- **The product/protocol spec of sirji-the-app**, and within it the public-places
  problem.
- **Per-requester device addresses** — the invariant is decided at the person
  tier; v1 uses one device id52 per device, with the address-pool upgrade path
  recorded above.
- **Ticket freshness, expiry, and revocation.** The sealed-ticket flow is
  decided; its lifetime mechanics are not.
- **The name-claim ceremony's exact mechanics** — how a device's first claim is
  authorised in practice.
- **Handshake-key rotation.** Now that a handshake key *is* the address, retiring
  one strips every peer that knows it, and a private key gives them no discovery
  path back. The options are keeping the old key bound in parallel until peers
  roll forward — the acknowledgement machinery this design deliberately dropped —
  or accepting that rotation means redistributing by hand. Not settled. Rarely
  needed, and painful when it is.
- **Horizontal scaling of a central sirji.**
- **The full traversal machinery** — multi-hop introduction chains, trust
  assertion formats, revocation. Per-relationship keys and single-hop introduction
  are v1; the deep graph tooling is not.
