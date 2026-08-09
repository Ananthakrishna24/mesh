# ADR-0001: Direct QUIC Transport with Quinn

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-08-09 |
| Owners | Architecture discussion |

## Context

The mesh must connect PCs directly across the internet. It must not depend on a public gateway, relay, permanent coordinator, or master node.

The transport needs:

- Incoming and outgoing connections from the same node.
- Reliable control and bulk-data streams.
- Several independent streams without one blocked transfer stopping all control traffic.
- UDP support for direct NAT traversal attempts.
- Connection recovery when a local network address changes.
- A maintained Rust implementation.

## Decision

Use Quinn as the Rust QUIC implementation. Run it on Tokio. Each mesh node owns one Quinn endpoint backed by a UDP socket.

Use:

- One long-lived bidirectional control stream per peer connection.
- Separate streams for large work and result transfers.
- QUIC datagrams only for data that may be lost safely.

Canonical algorithm: [Direct peer connection](../architecture/networking/direct-connection.md)

## Why this choice

Quinn provides a portable Rust QUIC implementation. Its endpoint supports incoming and outgoing connections. QUIC already provides stream multiplexing, flow control, congestion control, reliability, and transport encryption.

This keeps the mesh transport small while avoiding a custom reliable-UDP protocol.

## Rejected: raw TCP

TCP is simple, but direct connection attempts and address changes are less suitable than a UDP-based QUIC endpoint. A large transfer can also delay unrelated data when everything shares one TCP byte stream unless the project builds more connection management.

## Rejected: raw UDP

Raw UDP offers maximum control. It does not provide reliable delivery, ordering, congestion control, flow control, or independent streams. Implementing these correctly would recreate a large part of QUIC.

## Rejected: rust-libp2p as the base stack

rust-libp2p provides QUIC, peer addressing, discovery, AutoNAT, relays, and DCUtR hole punching. It is useful when those interoperable protocols are requirements.

Its standard DCUtR flow requires a relay for the initial coordination path. A relay conflicts with the accepted architecture. Pulling in the larger behaviour stack while replacing that path does not improve the first implementation.

The project may study libp2p protocols. It will not use rust-libp2p as its first network foundation.

## Consequences

- QUIC transport encryption exists even though encryption is not a current product concern. It is part of QUIC and will not be removed.
- The project owns invite encoding, peer exchange, peer-assisted introductions, and connection policy.
- QUIC does not solve unreachable CGNAT peers. Those peers remain unavailable when every direct method fails.
- Transport performance must be measured over real internet paths before model partitioning is designed.

## Sources

- [Quinn repository](https://github.com/quinn-rs/quinn)
- [Quinn `Endpoint` API](https://docs.rs/quinn/latest/quinn/struct.Endpoint.html)
- [rust-libp2p DCUtR example](https://github.com/libp2p/rust-libp2p/tree/master/examples/dcutr)
