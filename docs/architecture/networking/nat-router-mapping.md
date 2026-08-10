# NAT and Router Mapping

| Field | Value |
|---|---|
| Status | Accepted |
| Canonical for | Automatic router port mapping crates, mapping lifecycle, and Quinn socket binding |
| Parent | [Direct peer connection](direct-connection.md) |
| Decision | [ADR-0013: NAT and router mapping crates](../../decisions/0013-nat-router-mapping-crates.md) |
| Related | [Enrollment contract](../onboarding/enrollment-contract.md) |
| Implements gate | A08 |

## Boundary

`mesh-net` owns automatic router mapping and the Quinn UDP socket. Mapping never becomes a relay, rendezvous, or central service. When every automatic method fails, the enrollment GUI keeps the existing guided manual UDP forwarding path.

## Selected crates

| Protocol | Crate | Feature / module | Role |
|---|---|---|---|
| UPnP IGD | [`igd-next`](https://crates.io/crates/igd-next) | `aio_tokio` | Discover the gateway and add or delete a UDP port mapping |
| NAT-PMP | [`crab_nat`](https://crates.io/crates/crab_nat) | `crab_nat::natpmp` | RFC 6886 UDP mapping |
| PCP | [`crab_nat`](https://crates.io/crates/crab_nat) | `crab_nat::pcp` and `PortMapping::new` | RFC 6887 UDP mapping; preferred before NAT-PMP |

Gateway and local LAN address discovery stay inside `mesh-net`. Do not take a whole peer-to-peer stack only to open one UDP mapping.

## Rejected wrappers

- **`portmapper` (n0/iroh):** useful reference, but it pulls `iroh-metrics`, `netwatch`, and related workspace assumptions, and currently requires a newer Rust toolchain than this workspace. Prefer the thin protocol crates above.
- **Original `igd`:** superseded by `igd-next`.
- **Standalone `natpmp` only:** overlaps `crab_nat` and does not cover PCP.

## Quinn socket contract

Automatic mapping must target the same local UDP port Quinn uses.

Required bind sequence:

1. Create one `std::net::UdpSocket` bound to the configured listen address (default `0.0.0.0:0` or an explicit port).
2. Read the bound local port.
3. Run mapping attempts against that internal UDP port.
4. Build the Quinn endpoint with `quinn::Endpoint::new` (or equivalent) on the **already-bound** socket.
5. Publish candidates, including any successful external `RouterMapping` address.

Rules:

- Do not bind Quinn first on an anonymous port and then map a different port.
- Do not open a second permanent UDP socket for mesh traffic.
- Mapping control packets may use short-lived sockets to the gateway. Mesh accept/connect traffic uses only the Quinn socket.
- `Endpoint::rebind` is reserved for later local-address changes. The first P04 path does not require rebind.
- A unit test must prove two mesh endpoints can complete `HELLO`/`WELCOME` when each endpoint is constructed from a pre-bound `std::net::UdpSocket`.

## Mapping attempt order

While preparing candidates:

1. Collect local, global IPv6, and public IPv4 candidates as already defined.
2. Discover the default IPv4 gateway and the local IPv4 address used to reach it.
3. Attempt protocols in this order, stopping at the first successful UDP mapping that reports an external IPv4 address and port:
   1. PCP
   2. NAT-PMP
   3. UPnP IGD
4. If a protocol is unavailable or times out, try the next.
5. If all fail, continue enrollment without a `RouterMapping` candidate and keep manual forwarding available.

IPv6 global addresses do not require IPv4 router mapping. Mapping remains an IPv4 NAT tool.

## Mapping parameters

| Parameter | Value |
|---|---|
| Protocol | UDP only |
| Requested lifetime | 7,200 seconds |
| Renew when remaining lifetime | ≤ 50% |
| External port preference | Same as internal port; accept gateway-assigned port otherwise |
| Discovery deadline | 2 seconds per protocol |
| Full mapping budget during enrollment prep | 6 seconds |
| Description / lease label where supported | `mesh` |

On renewal failure:

1. Retry once immediately.
2. Retry once more after 30 seconds.
3. Drop the `RouterMapping` candidate and publish an updated candidate set.
4. Do not tear down existing QUIC sessions solely because renewal failed.

On shutdown or leave-mesh:

1. Best-effort delete the mapping.
2. Ignore delete failures after a short timeout.

## Candidate publication

A successful mapping adds:

```text
EndpointCandidate {
  kind: RouterMapping,
  address: external_ip:external_port,
  priority: CandidateKind::RouterMapping priority,
  observed_at_unix_ms: mapping success time,
  expires_at_unix_ms: lease end,
  source_node_id: local node,
  reachability: Unknown until a peer connects through it
}
```

Do not claim the mapped address is reachable until a peer successfully uses it.

## Failure and fallback

Mapping failure is normal on CGNAT, locked-down routers, and networks without IGD/PMP/PCP.

Behavior:

- Enrollment continues with the remaining candidates.
- The GUI may show that automatic router setup did not succeed.
- The recovery order stays the one in the [enrollment contract](../onboarding/enrollment-contract.md): retry candidates, refresh mapping, regenerate invitation, then guided manual UDP forwarding.
- Manual forwarding remains user-driven. The application never silently disables the host firewall.

## Security limits

- Only the mesh listen port is mapped.
- Only UDP is requested.
- Mapping is local-LAN gateway control traffic, not peer data relay.
- No third-party STUN, TURN, or hosted rendezvous service is introduced by this decision.

## Ownership

| Concern | Owner |
|---|---|
| Crate integration, renewals, candidate emission | `mesh-net` |
| Candidate merge and expiry | Peer Store rules in [Peer-record merge rules](peer-record-merge.md) |
| User-facing mapping progress and manual fallback copy | enrollment/GUI contracts |
| Quinn endpoint construction from the bound socket | `mesh-net` |

## P04 implementation checklist

- Depend on `igd-next` (`aio_tokio`) and `crab_nat` from `mesh-net`.
- Bind the UDP socket before mapping and before Quinn endpoint construction.
- Emit `RouterMappingCreated` only after a mapping result is accepted.
- Keep unit tests independent of a real consumer router; use a pre-bound socket proof plus mocked or ignored gateway failures.
