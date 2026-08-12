# sirji

An open-source peer-to-peer network substrate — identity, relationships, and
authenticated connections — shipped as a Rust crate that an app embeds to become
a sirji device.

Every actor is an ed25519 keypair. Identity is **pairwise**: a fresh keypair per
relationship, so the id52 a peer holds for you is an edge, not a node, and nothing
two peers hold can be correlated. Connections are QUIC over
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
- **[patterns/](patterns/)** — conventions for apps built on sirji, not substrate.
- **[road-ahead.md](road-ahead.md)** — beyond v1; vision, not scope.

## License

[MIT](LICENSE) © 2026 Amit Upadhyay
