# The road ahead

Where sirji could go beyond v1. **Not a roadmap, not a commitment, not v1 scope.**
v1 is deliberately built to grow into this without breaking anything.

Kept separate from [DESIGN.md](DESIGN.md) on purpose: the design is a fork-free
build spec, and nothing here is decided.

---

## Layer-2 — sync, storage, offline delivery

v1 is Layer-1 only (see [overview.md](overview.md) § Scope). The social and chat
surface cannot exist without durable, ordered, offline-tolerant message delivery,
and none of that is designed.

An earlier design of this substrate worked a good deal of it out — lifetime
subscriptions, parent-tracked push so a node only receives changes it asked for,
conflicts surfaced as ordinary events for the application to resolve, and
per-device capacity envelopes so a low-powered device can declare what it can
absorb. That material is the natural starting point.

What Layer-1 already owes Layer-2 is written down as a contract in DESIGN.md, so
this work can land without a wire change.

## Public places

Addressing already reaches them: a public place is a group sirji with a published
handshake key, and a stranger who knocks arrives with no alias at all, which is
exactly what anonymous participation needs.

What is unsolved is **shared mutable state** — the ordering of a thread many peers
write to, who stores its history, who serves it to a newcomer, who moderates, and
who pays for the storage and bandwidth. None of that is an addressing question.
It is Layer-2 plus app design, and it is where the application work will start.

## Transport beyond commercial pipes

The wire is iroh QUIC today, over whatever the internet provides. The substrate is
transport-agnostic by construction: identity is a keypair, not an address, so
nothing above the wire cares whether packets crossed commercial fibre, a
community-owned mesh, or a long-range radio relay.

Owning your own computer matters less if the only network it can speak over is a
single commercial pipe that can be shaped, throttled or tapped by parties you have
no relationship with. Community-owned RF backhaul — LoRa and sub-GHz mesh on
license-exempt bands, on hardware that already costs very little — is a real
long-term possibility. It needs RF engineering, mesh routing, regulatory and
civic-infrastructure expertise that the author does not have, which is why it is
articulated here rather than planned: so that people who *do* have it know there is
a project their knowledge connects to.

Intermediaries are welcome and never required. A commercial provider that wants to
participate speaks the same protocol as everyone else; it simply gets no
privileged position.

## The privacy stance

Not privacy zealotry, and worth being explicit because the language of
community-owned infrastructure can be read the wrong way.

sirji does **not** try to make *targeted* investigation impossible. Sufficient
targeted effort defeats what it provides — a seized device can be imaged, a hosted
node can be served process, traffic timing can be correlated — and promising
otherwise would be dishonest.

What it tries to make expensive is **mass** correlation. Pairwise identity defeats
trivial cross-relationship linking. The absence of a directory means there is no
namespace to enumerate. No intermediary sits at a point where everyone's traffic
can be tapped at once.

Targeted stays feasible. Dragnets get expensive. That is the line, it is
deliberate, and the technical choices follow from it.
