# Pattern — `network.md`, the app's model of the org

**Status: a convention, not substrate.** The sirji crate neither reads nor ships
this file. It is a recommended pattern for apps built on sirji, written down
because it is expected to recur.

Not to be confused with [`network.toml`](../DESIGN.md#networktoml--sirjis-known-net),
which *is* substrate: sirji's known net, deterministic, cryptographic identity
only.

---

## The problem it solves

sirji is deliberately thin. It resolves an incoming id52 to an
**alias** and hands that over in the sealed ticket — and that is *all* it hands
over. **It has no groups, no tags, and no visibility rules**: a group is not a
cryptographic fact, it is a name someone made up, so it has no place in the store
of cryptographic facts. sirji's only gate is possession of a handshake key.

Organisations are not thin. They have groups of groups, ad-hoc groups that exist
for a fortnight, roles that are not membership, on-call rotations, cost centres,
seniority that grants exceptions, teams that own services jointly. Every org
shapes these differently, and **no static schema anyone designs will fit the next
org.** Pushing that into `network.toml` would either bloat the access gate or,
worse, make it non-deterministic.

**So all grouping and aliasing lives here, one layer up, where a model reads it**
— and that is the point, not a concession. Freed from a schema, naming can be as
rich as the org's own vocabulary: a group defined by who is on call, an alias that
means "whoever is shipping this week", a set that expires. None of that is
expressible in static config, and all of it is ordinary prose here.

**So this file is not a convenience layer — it is where authorization lives.**
The substrate admits anyone holding a key you handed out and tells the app only
who they are. Every question of the form *may this person do this* is answered
here and in the app, or it is not answered at all.

## The shape

Three files, three jobs. The app owns the second and third:

| file | owner | read by | answers |
|---|---|---|---|
| `network.toml` | the sirji | the crate, deterministically | *whose key is this, and what is it called?* |
| `network.md` | the org | an LLM, semantically | *who is this person, organisationally?* |
| `policy.md` | the org | an LLM, semantically | *what are they allowed to have?* |

`policy.md` **refers to** `network.md` rather than restating it. Policy stays
about rules; the org model stays in one place.

**The alias is the join key.** sirji ships an alias and nothing else; the app looks
the alias up in `network.md`. Neither file knows the other's internals — they
meet at a name. This is what lets the same `network.md` serve several sirji apps
at once.

## What goes in it

Whatever the org needs, in the org's own vocabulary. The example below is one
org's answer, not a schema to conform to:

```markdown
# network.md — acme

## Teams

- **platform** — owns the worker fleet and this file. dana, kiran.
- **payments** — owns checkout and billing. Members rotate in from platform
  during a freeze; treat whoever is on the payments rota as a member for the
  duration.
- **qa** — includes everyone in engineering, plus lee and the contractors.

## Roles

- **release-captain** — rotates weekly, currently kiran. Not a team; a hat.
  A release-captain speaks for whichever team is shipping.
- **incident-commander** — whoever declared the current incident. Authority is
  scoped to that incident and ends with it.

## Ad-hoc groups

- **q3-migration** — a temporary group for the database cutover: dana, kiran,
  two payments engineers. Expires end of September; after that treat members by
  their standing teams.

## How to read seniority

Anyone in platform may speak for engineering in an emergency. Contractors may
not, regardless of team, unless a release-captain vouches in the request.
```

Then `policy.md` refers to it:

```markdown
## Standing budgets

- engineering: 50 parallel workers daily.
- Anyone the network model treats as on the payments rota during a freeze gets
  the payments cap, not their standing team's.
- A **release-captain** may exceed their team's cap by 2× while shipping.
```

Note what that buys: the budget rules never enumerate people, and the org model
never mentions worker counts. Reorganise the company, edit one file.

## Why an LLM-read config file can be trusted

The obvious objection: config interpreted by a model is config that can drift.
The answer is the one that makes any code trustworthy — **tests.**

These `.md` files are configuration with unit tests, which makes them
*effectively code*: strict and semantic, even though every org invents its own
semantics. Assertions turn "static enough for a model to infer reliably" from a
hope into a checked property:

```
assert  kiran           is-member-of  platform
assert  lee             is-member-of  qa
assert  contractor-x    not           speaks-for  engineering
assert  request("urgent, checkout down", from: lee)     ⇒  denied
assert  request("release", from: kiran)                 ⇒  granted, workers <= 100
```

Run them in CI against the real model. A failing assertion means the file is
ambiguous — the remedy is to make the prose more precise, never to loosen the
assertion.

**Write the tests when you write the file.** An `.md` config with no assertions
is prose, and prose drifts.

## Rules of thumb

1. **Nothing gates a connection except a key you handed out.** `network.toml`
   holds no access rules to move things into — so if a decision is about *what
   someone may do*, it is this file's and the app's, always.
2. **Expect unnamed peers.** Anyone arriving through a published handshake key has no
   alias. Decide explicitly what a stranger may do rather than assuming every
   caller is in the model.
3. **Name people once.** Aliases come from `network.toml` via the ticket; do not
   restate id52s here, ever. This file should contain no cryptographic material
   at all.
4. **Invent your own semantics, then test them.** There is no schema to conform
   to. There is an obligation to be checkable.
5. **One org model, many apps.** Keep it about the org, not about the app, so the
   next sirji app can read the same file.

## For other apps

The pattern generalises to anything that makes judgement calls about people over
sirji: a compute allocator, an approval flow, a support router, an on-call
escalator, a document gateway. The substrate hands you a name; `network.md` tells
you who that is; your `policy.md`-equivalent decides what happens. Only the third
file changes per app.
