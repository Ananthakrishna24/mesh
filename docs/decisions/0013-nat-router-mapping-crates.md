# ADR-0013: NAT and Router Mapping Crates

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-08-10 |
| Owners | Architecture discussion |
| Gate | A08 |

## Context

P04 must open automatic direct paths across consumer NATs when the router allows it. The direct-connection algorithm already requires UPnP, NAT-PMP, or PCP before guided manual forwarding. Quinn provides the QUIC endpoint but not gateway mapping. The project needs explicit Rust crates and a socket-binding rule so the mapped external port is the port Quinn actually owns.

## Decision

Accept the NAT and router mapping contract in [NAT and router mapping](../architecture/networking/nat-router-mapping.md).

Key rules:

- UPnP IGD uses `igd-next` with the `aio_tokio` feature.
- NAT-PMP and PCP use `crab_nat`.
- Attempt order is PCP, then NAT-PMP, then UPnP.
- Bind one `std::net::UdpSocket`, map that internal UDP port, then construct Quinn with `Endpoint::new` on the same socket.
- Mapping failure is non-fatal; guided manual UDP forwarding remains the fallback.
- No STUN, TURN, or hosted relay is introduced.

## Rejected: `portmapper` as a hard dependency

`portmapper` already combines UPnP, PCP, and NAT-PMP and is proven inside iroh. It also pulls iroh-adjacent metrics and network-watch crates and currently expects a newer Rust toolchain than this workspace. The protocol crates underneath are enough for P04 and keep the dependency surface small.

## Rejected: map after Quinn binds an opaque port without sharing the socket

If mapping and Quinn do not share the bound UDP port, peers dial an external port that nothing accepts. The pre-bound socket sequence is mandatory.

## Rejected: STUN-only public address discovery as the primary path

STUN can reveal a mapped address without controlling the gateway mapping lifetime. It also introduces an external infrastructure dependency. Optional future measurement tools may revisit discovery, but A08 selects local gateway control protocols only.

## Consequences

- `mesh-net` gains router-mapping modules and candidate emission for `RouterMapping`.
- Workspace dependencies add `igd-next` and `crab_nat` when P04 implementation starts.
- Tests must prove Quinn accepts pre-bound sockets even when no consumer router is present.
- Enrollment copy already describing automatic router setup and manual fallback stays valid.
