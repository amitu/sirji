# sirji

An open-source peer-to-peer network substrate — identity, relationships, and
authenticated connections — shipped as a Rust crate that an app embeds to become
a sirji device.

Every actor is an ed25519 keypair, and there are two kinds. A **handshake key** is
an address: what you listen on and hand out, rotatable, shared with whoever you
choose. A **peer key** is an identity: minted per relationship, shown to exactly
one peer, and only ever dialled *from* — so no two peers can correlate you, no
matter how many share your address. Connections are QUIC over
[iroh](https://github.com/n0-computer/iroh), mutually authenticated by keypair,
NAT-traversed and relay-backed, so location never matters.

There is no directory, no global namespace, and no account. You reach what you
were given a handshake key to knock on, and nothing else.

**Status: design settled, implementation starting.** The protocol is decided; the
social/chat application on top of it is deliberately not designed yet.

- **[overview.md](overview.md)** — what sirji is for, and what Layer-1 does and
  does not cover.
- **[DESIGN.md](DESIGN.md)** — the substrate, decided: id52 identity, the entity
  model, `network.toml`, keys on disk, registration and liveness, the
  control/data-plane connection flow, bootstrap, `name@sirji` addressing, and the
  build order.
- **[PLAN.md](PLAN.md)** — the implementation plan: two spikes that gate the
  design, then seven milestones, each ending in something runnable.
- **[patterns/](patterns/)** — conventions for apps built on sirji, not substrate.
- **[road-ahead.md](road-ahead.md)** — beyond v1; vision, not scope.

## License

Licensed under either of [MIT](LICENSE-MIT) or
[Apache-2.0](LICENSE-APACHE) at your option. © 2026 Amit Upadhyay

Unless you state otherwise, any contribution you intentionally submit for
inclusion in this work shall be dual licensed as above, without any additional
terms or conditions.
