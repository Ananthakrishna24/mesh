# Direct Peer Connection

| Field | Value |
|---|---|
| Status | Accepted |
| Canonical for | How PCs find and connect directly to each other |
| Parent | [Architecture overview](../README.md) |
| Decision | [ADR-0001: Quinn QUIC transport](../../decisions/0001-direct-quic-transport.md) |

## Decision summary

Use one Quinn QUIC endpoint per node. It can accept incoming connections and create outgoing connections over one UDP socket. QUIC is the direct transport. There is no compute-data relay.

This is the most efficient practical starting point for this project:

- It avoids the extra layers of a general peer-to-peer framework.
- It provides reliable streams, congestion control, and connection recovery.
- It allows control messages and large compute transfers to use separate streams.
- It avoids building reliability and packet ordering on raw UDP.
- It is better suited to hole punching than a new TCP connection.

It is not the theoretical fastest transport on every network. Raw UDP could remove some work, but the project would then need to rebuild loss recovery, ordering, congestion control, and flow control. That would add risk without improving the first hardware mesh.

QUIC always encrypts its transport. The project does not add a separate encryption layer in this phase.

## Required local data

```rust
struct LocalNode {
    mesh_id: MeshId,
    node_id: NodeId,
    protocol_major: u16,
    protocol_minor: u16,
    candidates: Vec<EndpointCandidate>,
}

struct EndpointCandidate {
    kind: CandidateKind,
    address: std::net::SocketAddr,
    priority: u16,
}

enum CandidateKind {
    GlobalIpv6,
    PublicIpv4,
    RouterMapping,
    Manual,
    PeerObserved,
    LocalNetwork,
}
```

These types describe local state. Their wire representation follows the accepted [control protocol](../protocol/control-protocol.md).

## A. Start a node

1. Load or create the persisted certificate and derive its stable `NodeId`.
2. Load or create the `MeshId`.
3. Bind one UDP socket.
4. Create one Quinn endpoint on that socket.
5. Start accepting incoming QUIC connections.
6. Collect candidate addresses.
7. Store the candidates in local Node State.

The socket port is configurable. A fixed default port may be added later.

## B. Collect candidate addresses

Try these sources in order:

1. Globally reachable IPv6.
2. Public IPv4 already assigned to the PC.
3. Router mapping through UPnP, NAT-PMP, or PCP.
4. A manually configured public address and forwarded port.
5. An address reported by an already-connected peer.
6. Local network addresses for peers on the same network.

Do not label a candidate as reachable until a peer successfully uses it.

## C. Create an invite

The invite contains the inviter identity, Mesh ID, protocol range, one-time enrollment values, expiry, and current endpoint candidates.

Its exact Protobuf payload, `mesh1:` text, `.mesh-invite` file, `mesh://` URI, and QR encoding are defined by the [Enrollment contract](../onboarding/enrollment-contract.md).

## D. Join through the inviter

1. The joining node parses the invite.
2. It checks the invite format version.
3. It sorts candidates by priority.
4. It starts with global IPv6 and public IPv4.
5. It tries lower-priority addresses after a short delay.
6. The first successful QUIC connection wins.
7. Remaining attempts are cancelled.
8. The nodes run the mesh handshake.

Trying likely addresses with a small delay is faster than waiting for each failed address to time out.

## E. Mesh handshake

The first bidirectional QUIC stream is the control stream. Its exact framing, `HELLO`, `WELCOME`, version negotiation, errors, and limits follow the [control protocol](../protocol/control-protocol.md).

The TLS certificate supplies the sender identity. Its SHA-256 digest must equal the `sender_node_id` in every control envelope.

During first enrollment, the inviter identity must match the invitation and the joining peer must present its enrollment ID and secret. During reconnection, both Node IDs and certificates must already match Peer Store.

A failed check sends a typed control error when possible and closes the connection.

## F. Connect to the remaining peers

1. The joining node writes the peer snapshot to Peer Store.
2. It removes itself and the already-connected inviter.
3. It dials candidate addresses for the remaining peers.
4. Each successful peer runs the same handshake.
5. Each peer shares newer peer records it knows.
6. The joining node now has direct sessions with reachable peers.

The inviter may disconnect after this. It has no permanent role.

## G. Peer-assisted hole punching

This is possible only after both blocked peers already share a connection with another peer.

Example: PC A and PC B are both connected to PC C.

1. PC A asks PC C for an introduction to PC B.
2. PC C sends A's observed UDP address to B.
3. PC C sends B's observed UDP address to A.
4. PC C gives both PCs the same short-lived attempt ID and start time.
5. A and B send UDP probes to each other at that time.
6. Their routers may open a direct path.
7. A and B start a Quinn connection on the successful path.
8. A and B perform the normal mesh handshake.
9. Compute traffic moves directly between A and B.

PC C carries introduction messages only. It does not carry compute data.

This method will not work through every router. Standard rust-libp2p DCUtR uses a relay for its initial coordination path, so it does not satisfy this project's no-relay rule.

## H. Duplicate connection rule

Two peers may dial each other at the same time.

After both connections complete the handshake:

1. Compare both `NodeId` values.
2. The peer with the lower `NodeId` is the preferred connection initiator.
3. Keep the connection initiated by that peer.
4. Close the duplicate connection.

If only one connection works, keep it regardless of who initiated it.

## I. Connection state

```text
DISCONNECTED
    │
    ▼
GATHERING_ADDRESSES
    │
    ▼
DIALING ───── failure ─────▶ BACKOFF
    │                           │
    ▼                           └────▶ DIALING
HANDSHAKING
    │
    ▼
CONNECTED ─── link lost ───▶ BACKOFF
```

Reconnect delays should use capped exponential backoff with jitter. Exact timing remains proposed until network tests provide data.

## J. QUIC stream use

| Stream | Direction | Use |
|---|---|---|
| Control | Bidirectional, long-lived | Length-prefixed Protobuf handshake, peer updates, health, and job control |
| Activation | Unidirectional, one stream per tensor | Fixed header and raw inference activation |
| Artifact or work | One stream per transfer | Model data, tensor chunks, datasets, or other job input |
| Result | One stream per transfer | Large job results and outputs |
| Datagram | Unreliable | Optional hole-punch probes and disposable measurements |

Do not place a large model transfer on the control stream. It would delay health and job-control messages.

## Unavoidable failure case

If a new node and the inviter are both unreachable, they cannot create the first connection. At least one initial peer needs global IPv6, public IPv4 with forwarding, automatic router mapping, or another working direct path.

No software-only algorithm can guarantee direct communication through every CGNAT and firewall without an outside rendezvous or relay service.

## Open questions

- Full-mesh peer-count limit.

Router-mapping crates and Quinn pre-bound socket rules are accepted in [NAT and router mapping](nat-router-mapping.md).
Candidate expiry and peer-record merge rules are accepted in [Peer-record merge rules](peer-record-merge.md).
Network benchmark thresholds used by job scheduling are accepted in [Network benchmark and placement cost](network-benchmark.md).
