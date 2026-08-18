# Standing: how a stranger acquires the right to do something

A pattern for apps built on sirji. Not substrate — sirji answers *which key am I
talking to, and do I have a relationship with it*, and stops there.

Most interesting apps need a second question answered:

> **On whose behalf is this key acting, and who says so?**

That is standing, and this is the shape it takes.

## Why the substrate does not answer it

sirji's model is pairwise: two durable parties, each holding a key, each having
seen the other's. That is exactly right for one-to-one chat and for your own
constellation of devices, and it is the wrong shape for everything else.

Look at what actually turns up:

| app | shape |
|---|---|
| 1:1 chat | two durable parties, symmetric, mutual |
| your own devices | one identity, many devices |
| an allocator of shared machines | one party owns a resource, many consume it |
| a public place | one party owns the place, many participate, most are strangers |

The last two are the same shape: **a resource with an owner, and others acquiring
standing on it.** Neither is pairwise-symmetric. A relationship with ten thousand
strangers is not a relationship.

## The four ways standing is acquired

There are only four, and every app picks a subset.

**Anonymous.** A stranger knocks on a published handshake key and arrives with no
alias. This already works and is not an error condition — it is what participation
without prior contact looks like. Suitable for reading, for rate-limited action, or
for anything paid per use.

**A capability.** Someone who holds rights hands you a scoped, revocable token: an
invite link to a room, a pre-minted grant. sirji's sealed ticket is one of these —
*"whoever holds key K may talk to device D as alias A until T"* — and the pattern
generalises it.

**An attestation.** A third party the owner already trusts vouches for you. An
OIDC token from a CI provider, a session from an identity provider, or socially,
three existing members vouching.

**A relationship.** The native case, for parties durable enough to have one.

## The ticket is an attestation with a particular issuer

Worth seeing clearly, because it stops the other mechanisms looking like
concessions. A sealed ticket is *a statement by an issuer about a key*. sirji's
daemon is one issuer. An identity provider is another. A CI platform is a third.

| issuer | statement |
|---|---|
| a sirji daemon | "K may act as our device, or as our peer dana" |
| an OIDC provider | "this run is repo acme/foo on ref main" |
| an account system | "this is user dana in org acme" |
| a shared secret | "the holder is acme's CI" |

Same shape throughout. The last one is the **weakest**, because the claim and the
proof are the same bytes, and it should be the last resort rather than the default.

## When a shared secret is genuinely required

Almost never, and it is worth checking before reaching for one. A secret is only
needed when an actor is **both ephemeral and unattested**:

| actor | ephemeral | attested |
|---|---|---|
| cloud CI runner | yes | yes — the platform mints a token |
| developer laptop | no | — |
| on-prem build server | **no** | — |
| a long-running service | no | — |

The intersection is empty. Anything ephemeral is attested by the platform that
created it; anything unattested is durable enough to hold a key. The mistake is
classifying by category — "CI needs secrets" — rather than by the property that
matters, which is **continuity**. A build server that has run for three years is
not ephemeral, whatever it is called.

## Ephemeral actors should not be given identities

A CI runner has no past and no future. Giving it a durable keypair models a
fiction, and the fiction has a cost: a roster entry per build.

The accurate model is that **the organisation is the identity and the runner is a
momentary agent of it.** So: generate a keypair per run, discard it, and let an
attestation say whose agent this is. Nothing is enrolled, so nothing accumulates.

Bind the attestation to that one-off key where the issuer allows it — many OIDC
providers let the caller choose the audience, so putting the ephemeral public key
there makes the token useless to anyone else. That recovers the caller-binding
property a sealed ticket has, for a credential the substrate never issued.

## Enrolment: how a durable actor gets a key recognised

For anything durable, the answer is a keypair, and the only question is how it
becomes known. That is enrolment, and it happens **through the anonymous door**.

A stranger arriving on a published handshake key has no alias and may do exactly
one thing: attempt to enrol. Everything else is refused. No second channel, no
HTTP endpoint beside the sirji one.

An app should advertise the proofs it accepts rather than the client hardcoding
providers:

```json
{ "methods": [
  { "kind": "oidc-device", "issuer": "https://accounts.example.com", "client_id": "…" },
  { "kind": "token",       "prompt": "access key" },
  { "kind": "invite",      "hint":   "ask your operator for `sirji device invite`" }
]}
```

The client then implements **protocols, not vendors** — one RFC 8628 device-flow
implementation covers every standards-compliant provider, discovered through
`/.well-known/openid-configuration`. Adding a provider is server-side
configuration.

Two rules worth keeping:

- **Mint a fresh keypair per service.** The substrate mints a peer key per
  relationship so no two peers can correlate you; the same reasoning applies to
  services, and the same keystore already stores keys this way.
- **After enrolment there is no session.** No token, no expiry, no refresh —
  every later request is keypair-authenticated. Revocation is the service
  forgetting the key, which is also how a member is removed from a place.

## Declared and attested must stay apart

Whatever an app lets a caller say about itself, it must keep separate from what
was proven:

```
attested   org, actor, repo, ref …    the caller cannot lie about these
declared   role, purpose, context …   what the caller says it is doing
```

Collapsing them into one trusted set feels safer and is worse: the **gap between
them is information**. A request declaring itself a nightly build while attested as
arriving from a pull request has told you something, and only an app that kept the
two apart can notice.

Keep the distinction visible all the way to wherever the decision is made.

## What this does not solve

Standing is who may act. It says nothing about **shared mutable state** — the
ordering of a thread many peers write to, who stores its history, who serves it to
a newcomer, who pays for the storage. That is Layer-2 and genuinely undesigned; see
`road-ahead.md`.

A request/response app needs standing and no Layer-2 at all, which is why this
pattern is worth writing down now.
