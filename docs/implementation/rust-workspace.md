# Rust Workspace Plan

| Field | Value |
|---|---|
| Status | Proposed |
| Canonical for | Initial Rust crate boundaries |
| Parent | [Documentation index](../README.md) |

No Rust workspace exists yet. This document defines the intended boundary before scaffolding.

## Workspace shape

```text
apps/
└── mesh-app/        Default eframe desktop package; binary name `mesh`
crates/
├── mesh-core/       Stable IDs, messages, shared errors, node state
├── mesh-net/        Quinn endpoint, peer sessions, address candidates
├── mesh-hardware/   CPU, memory, CUDA, and Metal discovery
├── mesh-model/      Provider adapters, manifests, partial downloads, cache
├── mesh-compute/    CUDA and Metal execution backends
├── mesh-inference/  Placement, reservations, batching, pipeline control
└── mesh-node/       Node runtime composition and lifecycle library
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

### `mesh-model`

Implements [provider-backed model distribution](../architecture/inference/model-distribution.md).

It owns provider adapters, immutable model identity, normalized manifests, Safetensors range planning, validation, and disk caching. Provider-specific types do not leave this crate.

### `mesh-compute`

Owns local compute backends. It may depend on CUDA, Metal, or model frameworks behind Cargo features.

It does not know how peers connect or where a stage was placed.

### `mesh-inference`

Implements [distributed LLM inference](../architecture/inference/README.md).

It owns placement planning, local resource reservations, model preparation coordination, request batching, pipeline control, and inference failure rules. It depends on platform-neutral types from `mesh-core`.

### `mesh-node`

Composes the node runtime as a library.

It:

1. Loads configuration.
2. Starts tracing.
3. Creates Node State.
4. Starts Hardware Scanner.
5. Starts Direct Link Manager.
6. Starts Job Manager, Local Resource Manager, Model Store, and Inference Worker when those phases begin.
7. Publishes typed state snapshots and progress events.
8. Handles graceful shutdown.

Business rules stay in the owning library crate. The GUI drives this runtime through typed commands.

### `mesh-app`

Implements [desktop onboarding](../architecture/onboarding/README.md) with `eframe`.

It is the default workspace package and produces the `mesh` executable. It starts the Tokio node runtime, sends UI commands, and renders state snapshots. It does not contain networking, model, or inference rules.

## Accepted base stack

| Need | Choice | State |
|---|---|---|
| Language | Rust stable | Accepted |
| Native desktop GUI | egui through eframe | Accepted |
| Async runtime | Tokio | Proposed with Quinn |
| Direct transport | Quinn QUIC | Accepted |
| NVIDIA discovery | NVML through a Rust wrapper | Proposed |
| Apple discovery | Native Metal bindings | Proposed |
| First inference engine | Candle | Accepted for proof; platform validation required |
| First model provider | Hugging Face Hub through `hf-hub` | Accepted |
| First model family | Dense Qwen3 4B and 8B | Accepted |
| First partial model format | Safetensors | Accepted |
| Training engine | Not selected | Deferred |

## Build targets

At minimum, build and verify:

- `x86_64-pc-windows-msvc` with NVIDIA CUDA.
- `x86_64-unknown-linux-gnu` with NVIDIA CUDA.
- `aarch64-apple-darwin` with Apple Metal.

Windows CUDA is a required native target. WSL-only execution does not satisfy it. Canonical decision: [ADR-0006](../decisions/0006-windows-nvidia-required.md).

## First implementation slice

The first runnable slice should do only this:

1. Run `cargo run --release` and open the native first-run window.
2. Create a mesh on the first PC through the GUI.
3. Generate an invitation with **Add another PC**.
4. Start the same application on a second PC.
5. Enroll the second PC by pasting or opening the invitation.
6. Bind a Quinn endpoint in each process.
7. Complete `HELLO` and `WELCOME`.
8. Exchange static test hardware reports.
9. Close and restart one application.
10. Reconnect automatically and open the dashboard.

No CLI arguments or separate helper processes are required. Real hardware probes should follow after this path works.
