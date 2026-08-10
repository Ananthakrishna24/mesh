# Architecture Overview

| Field | Value |
|---|---|
| Status | Accepted foundation |
| Canonical for | System boundary and topology |
| Parent | [Documentation index](../README.md) |

## Goal

Combine compute available on internet-connected PCs while keeping compute communication direct and decentralized.

## Current implementation boundary

The first implementation phase builds the hardware mesh. It must:

1. Start the same node software on every PC.
2. discover local hardware.
3. Join through one known peer.
4. Learn the remaining known peers.
5. Open direct internet connections.
6. Share hardware capabilities and node state.
7. Reconnect after a temporary network failure.

The accepted desktop onboarding and inference architecture are documented but not implemented in this phase:

- [Desktop onboarding](onboarding/README.md)
- [Enrollment contract](onboarding/enrollment-contract.md)
- [Persistent state and credentials](system/persistent-state.md)
- [Control protocol and versioning](protocol/control-protocol.md)
- [Distributed LLM inference](inference/README.md)
- [Provider-backed partial model distribution](inference/model-distribution.md)
- [Inference parallelism and edge cases](inference/parallelism-and-edge-cases.md)
- [Qwen3 dense 4B and 8B model family](inference/qwen3-model-family.md)
- [Activation tensor wire format](protocol/activation-frame.md)

Distributed training, gradient synchronization, and a public relay or control service remain outside the accepted inference design.

## Topology

```text
                       DIRECT LINK
                 ┌────────────────────┐
                 │                    │
                 ▼                    ▼
          ┌────────────┐       ┌────────────┐
          │  PC A      │◀─────▶│  PC B      │
          │  Mesh node │       │  Mesh node │
          └─────┬──────┘       └─────┬──────┘
                │                    │
                │    DIRECT LINK     │
                ▼                    ▼
          ┌────────────┐       ┌────────────┐
          │  PC C      │◀─────▶│  PC D      │
          │  Mesh node │       │  Mesh node │
          └────────────┘       └────────────┘
```

Every node is equal at the mesh level. A full mesh is acceptable for the first small deployment. A later decision must set the size where full-mesh connections stop being practical.

## Main flows

### First launch

One native Rust application starts the GUI and node runtime. It guides the user through creating a mesh or enrolling this PC with one invitation.

Canonical experience: [Desktop onboarding](onboarding/README.md)

### Join

A new PC receives an invite from one known peer. It connects to that peer, receives the known peer list, and then tries to connect directly to those peers.

Canonical algorithm: [Direct connection algorithm](networking/direct-connection.md)

Router mapping crates and Quinn socket binding: [NAT and router mapping](networking/nat-router-mapping.md)

Peer Store merge and candidate expiry: [Peer-record merge rules](networking/peer-record-merge.md)

### Hardware report

The Hardware Scanner reads local devices. Node State stores the result. The Node Connector shares a summary with connected peers.

Canonical modules: [Node modules](system/node-modules.md)

### Inference job

Any peer may create an inference deployment and become its temporary coordinator. It resolves one immutable model revision, reserves selected nodes, prepares model stages, and schedules requests.

Canonical architecture: [Distributed LLM inference](inference/README.md)

## Hard network truth

A direct connection cannot be guaranteed between every pair of consumer internet connections. If IPv6, public IPv4, port mapping, manual forwarding, and peer-assisted hole punching all fail, the two peers cannot connect. There is no relay fallback in this architecture.

## Performance truth

The mesh adds available machines. It does not automatically create one faster virtual GPU.

- Independent jobs can scale well.
- Long jobs with small inputs and outputs can scale well.
- A model may be split to increase total available GPU memory, but internet latency can make it slower.
- Work that synchronizes tensors many times per second will usually perform poorly over the public internet.

These facts must remain visible when inference and training are designed.
