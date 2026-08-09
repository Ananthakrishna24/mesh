# Architecture Overview

| Field | Value |
|---|---|
| Status | Accepted foundation |
| Canonical for | System boundary and topology |
| Parent | [Documentation index](../README.md) |

## Goal

Combine compute available on internet-connected PCs while keeping compute communication direct and decentralized.

## Current boundary

The current phase builds the hardware mesh only. It must:

1. Start the same node software on every PC.
2. discover local hardware.
3. Join through one known peer.
4. Learn the remaining known peers.
5. Open direct internet connections.
6. Share hardware capabilities and node state.
7. Reconnect after a temporary network failure.

The current phase does not implement:

- LLM loading or inference.
- Model partitioning.
- Distributed training.
- Gradient synchronization.
- Tensor placement across PCs.
- A public relay or control service.

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

### Join

A new PC receives an invite from one known peer. It connects to that peer, receives the known peer list, and then tries to connect directly to those peers.

Canonical algorithm: [Direct connection algorithm](networking/direct-connection.md)

### Hardware report

The Hardware Scanner reads local devices. Node State stores the result. The Node Connector shares a summary with connected peers.

Canonical modules: [Node modules](system/node-modules.md)

### Job

Any peer may create a job. That peer becomes the temporary owner of that job. It selects workers, sends work, tracks progress, and combines returned results.

The exact inference and training job formats are deferred.

## Hard network truth

A direct connection cannot be guaranteed between every pair of consumer internet connections. If IPv6, public IPv4, port mapping, manual forwarding, and peer-assisted hole punching all fail, the two peers cannot connect. There is no relay fallback in this architecture.

## Performance truth

The mesh adds available machines. It does not automatically create one faster virtual GPU.

- Independent jobs can scale well.
- Long jobs with small inputs and outputs can scale well.
- A model may be split to increase total available GPU memory, but internet latency can make it slower.
- Work that synchronizes tensors many times per second will usually perform poorly over the public internet.

These facts must remain visible when inference and training are designed.
