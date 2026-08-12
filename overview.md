# sirji — Overview

What sirji is for and how it is built. It does not specify the substrate — that is
decided in [DESIGN.md](DESIGN.md) — and it does not specify the product; the
social/chat features and UX remain deferred. Vision beyond v1 is
[road-ahead.md](road-ahead.md).

## What sirji is

sirji is an open-source **peer-to-peer network substrate** — identity,
relationships, and authenticated connections — and, in time, the flagship
social/chat app built on it. The substrate ships as the **sirji crate** (Rust); an
app embeds it and thereby *is* a sirji device.

Conceptually the destination is a decentralized, open-protocol social and
communication platform, intermediary-free, owned by no one in full control. The
substrate is decided. The social features that would make it that destination are
not.

## Scope — Layer-1

sirji v1 is **Layer-1**: who you are, who you know, and how a connection to a
named service gets established, authenticated and enriched. It ends the moment an
app holds an authenticated stream to the right device.

**Not in v1:** message sync, durable ordered storage, offline delivery,
replication, conflict resolution, file transfer. That is Layer-2, and it is
genuinely undesigned.

This boundary is deliberate, and worth stating plainly rather than discovering
later: **a request/response app needs no part of Layer-2** — it opens a stream and
sends structs — so Layer-1 alone is enough to ship something real. A chat or
public-place app needs all of Layer-2. So sirji is complete for the first kind of
app and roughly a third complete for its own flagship. Saying so now avoids
mistaking "the first app shipped" for "sirji is done".

DESIGN.md states what Layer-1 owes Layer-2 as a contract, so Layer-2 can be
designed later without a wire change.

## Why Rust, and why the substrate first

sirji is not a research exercise: it has a real consumer with a delivery date, an
allocator service that embeds the crate and ships. That is why the substrate is
built first, in plain Rust, with no dependency on anything unfinished.

It is also why Layer-1 was scoped the way it was. The first consumer exercises
identity, naming, liveness and authenticated connections hard, and needs no
durable messaging at all — so Layer-1 is exactly the part that a real workload
proved necessary, rather than the part that seemed interesting.

## A second implementation, later

A second, independent client for the same protocol is planned, in a different
language. That is not redundancy: **multiple independent implementations are what
prove a protocol is a protocol** rather than one program's internals. Anything
that turns out to be inexpressible outside the reference implementation is a bug
in the protocol, and the second implementation is how it gets found.

The crate survives that exercise unchanged; a second client re-expresses the
*logic*, not the wire.

## How we build it

Incrementally, minimal, verified by **running end to end** — not by unit tests
over isolated types. Every capability earns its place by being needed by something
real. The build order is in [DESIGN.md](DESIGN.md) § Build order.

## Design principles

Five commitments that explain most of the specific decisions:

1. **Pairwise identity, always.** No keypair is ever shown to two peers. Costs
   almost nothing, and makes cross-relationship correlation structurally
   impossible rather than merely discouraged.
2. **The substrate holds no policy.** It identifies; it does not authorize.
   Possession of a handshake key is the only gate, and every *may this person do
   this* question belongs to the app.
3. **Cryptographic facts are static; judgement is semantic.** They live in
   different files, read by different things, and never mix.
4. **No mechanism for a fact already established elsewhere.** A handshake key's
   distribution is its policy — a flag saying the same thing again is a second
   source of truth that can disagree with the first.
5. **Minimal is a feature.** Every concept removed is a concept nobody has to
   learn, implement twice, or keep consistent. Several things were cut from this
   design after being written; DESIGN.md records what and why so they are not
   re-proposed.
