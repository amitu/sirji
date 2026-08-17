# sirji

An open-source peer-to-peer network substrate — identity, relationships, and
authenticated connections — shipped as a Rust crate that an app embeds to become
a sirji device.

Every actor is an ed25519 keypair, and there are two kinds. A **handshake key** is
an address: what you listen on, published freely, rotatable, and shared with every
peer as a set. A **peer key** is an identity: minted per relationship, shown to
exactly one peer, and only ever dialled *from* — so it needs no address of its own,
and no two peers can correlate you even though your addresses are public. Connections are QUIC over
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
- **[t-sirji-fs/](t-sirji-fs/)** — the reference consumer: a device that serves a
  directory, which another peer can list and download from.
- **[docs/relay.md](docs/relay.md)** — pointing sirji at a relay, and why you will
  need your own. Deploying one is iroh's business, not ours.
- **[road-ahead.md](road-ahead.md)** — beyond v1; vision, not scope.

## When it doesn't work

```
$ sirji doctor
sirji doctor — /Users/dana/.sirji
  ok    home           1 address(es), 1 peer(s), 4 device(s)
  ok    keys           1 private key(s), all readable
  ok    daemon         running
  ok    dns            resolved dns.iroh.link, stun.l.google.com
  ok    udp egress     stun.l.google.com:19302 answered in 11ms
  FAIL  relay          https://aps1-1.relay.n0.iroh.link/ — tls connection failed:
                       invalid peer certificate: “FG1A0B2C3D4E5F67” is not trusted
                       → TLS was intercepted and the interceptor's CA is not
                         trusted here. Add it to the system store, or point
                         SIRJI_EXTRA_CA at the PEM.

1 problem above. UDP works, so this is fixable:
a relay you run — on a domain your network already trusts — is
usually the whole answer. Set SIRJI_RELAY to it.
```

Those two lines are the reason the command exists. **UDP got out in 11ms and every
relay hostname was blocked** — opposite conclusions, with opposite responses. One
says the network cannot carry QUIC at all; the other says the packets are fine and
only the coordination is filtered, so move it somewhere the filter has not
categorised. Nothing in a connection timeout tells you which you are looking at, and
working it out by hand takes an afternoon.

(The certificate above is named after a filtering appliance's serial number. That is
what interception looks like from the inside, and printing the whole error chain
rather than "failed to connect" is what makes it visible.)

`sirji doctor` needs no daemon — the moment you most want a diagnosis is when
nothing is running.

## License

Licensed under either of [MIT](LICENSE-MIT) or
[Apache-2.0](LICENSE-APACHE) at your option. © 2026 Amit Upadhyay

Unless you state otherwise, any contribution you intentionally submit for
inclusion in this work shall be dual licensed as above, without any additional
terms or conditions.
