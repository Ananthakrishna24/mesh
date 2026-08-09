# Rust Workspace Plan

| Field | Value |
|---|---|
| Status | Proposed |
| Canonical for | Initial Rust crate boundaries |
| Parent | [Documentation index](../README.md) |

No Rust workspace exists yet. This document defines the intended boundary before scaffolding.

## Workspace shape

```text
crates/
├── mesh-core/       Stable IDs, messages, shared errors, node state
├── mesh-net/        Quinn endpoint, peer sessions, address candidates
├── mesh-hardware/   CPU, memory, CUDA, and Metal discovery
├── mesh-compute/    GPU backend boundary; real execution comes later
└── mesh-node/       Binary that starts and connects all modules
```

Use a Cargo workspace at the repository root.

## Crate rules

### `mesh-core`

Contains data and rules that do not perform network or GPU input/output.

Allowed:

- `MeshId` and `NodeId`.
- Peer and hardware records.
- Connection state.
- Protocol messages.
- Shared error types.

Not allowed:

- Quinn types in public interfaces.
- CUDA or Metal types.
- Operating-system calls.

### `mesh-net`

Implements the [direct connection algorithm](../architecture/networking/direct-connection.md).

Expected dependencies:

- `tokio` for the asynchronous runtime.
- `quinn` for QUIC.
- A selected serialization crate after the wire format is accepted.
- Router-mapping crates after evaluation.

It depends on `mesh-core`. It does not depend on GPU crates.

### `mesh-hardware`

Implements the Hardware Scanner.

It exposes platform-neutral reports from `mesh-core`. NVIDIA and Metal integrations remain private and feature-gated.

### `mesh-compute`

Owns local compute backends. It is allowed to depend on CUDA, Metal, or model frameworks behind Cargo features.

It must not know how peers connect.

### `mesh-node`

Composition root only.

It:

1. Loads configuration.
2. Starts tracing.
3. Creates Node State.
4. Starts Hardware Scanner.
5. Starts Direct Link Manager.
6. Starts Job Manager and GPU Worker when those are implemented.
7. Handles clean shutdown.

Business rules should live in the owning library crate, not in the binary.

## Accepted base stack

| Need | Choice | State |
|---|---|---|
| Language | Rust stable | Accepted |
| Async runtime | Tokio | Proposed with Quinn |
| Direct transport | Quinn QUIC | Accepted |
| NVIDIA discovery | NVML through a Rust wrapper | Proposed |
| Apple discovery | Native Metal bindings | Proposed |
| First inference engine | Candle | Proposed |
| Training engine | Not selected | Deferred |

## Build targets

At minimum, design for:

- `x86_64-unknown-linux-gnu` with optional CUDA.
- `aarch64-apple-darwin` with optional Metal.

Windows CUDA support may be added, but it is not accepted as a first implementation target yet.

## First implementation slice

The first runnable slice should do only this:

1. Start two `mesh-node` processes.
2. Bind a Quinn endpoint in each process.
3. Connect using a manually supplied invite.
4. Complete `HELLO` and `WELCOME`.
5. Exchange static test hardware reports.
6. Disconnect one process.
7. Restart it and reconnect.

Real hardware probes should follow after the direct connection is proven.
