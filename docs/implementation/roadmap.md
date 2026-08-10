# Architecture and Implementation Roadmap

| Field | Value |
|---|---|
| Status | Active |
| Canonical for | Remaining decisions, implementation order, and phase proofs |
| Parent | [Documentation index](../README.md) |

## Current state

P01 shell, P02 enrollment, P03 Linux hardware/network discovery, P04 automatic direct connectivity, and P05 resource reservations are implemented. A07, A08, and A10 are accepted. Windows and macOS host proofs remain deferred to the end. Next implementation phase is P06 model provider and cache after A11–A13 are locked.


Accepted areas:

- Direct Quinn/QUIC peer connections.
- Decentralized peer topology.
- Native desktop onboarding.
- Local resource reservations.
- Single-node, replica, and layer-pipeline inference modes.
- Provider-backed partial model distribution.
- Dense Qwen3-4B complete-model proof and Qwen3-8B distributed proof.
- Required native NVIDIA CUDA support on Windows and Linux and Apple Metal support on macOS.

## Locked implementation contracts

- [Control protocol](../architecture/protocol/control-protocol.md): Protobuf through Prost, version `1.0`, fixed length framing, and typed errors.
- [Enrollment contract](../architecture/onboarding/enrollment-contract.md): certificate-derived Node IDs and exact self-contained invitation encoding.
- [Persistent state](../architecture/system/persistent-state.md): bundled SQLite with native provider credential stores.
- [Activation tensor frame](../architecture/protocol/activation-frame.md): fixed 128-byte header and contiguous little-endian FP16 payload.

## Remaining architecture decisions

Resolve these before their implementation phase begins.

### A01 — Qwen3 dense Model Family Adapter

The accepted family is dense Qwen3:

- `Qwen/Qwen3-4B` for complete-model and backend proofs.
- `Qwen/Qwen3-8B` for distributed layer-pipeline proof.

The adapter must:

- Identify Qwen3 from provider configuration.
- Map tensor names to layer ownership.
- Calculate per-layer memory.
- Build only an assigned continuous layer range.
- Handle embedding, final normalization, output head, and tied weights.
- Validate native Windows CUDA, Linux CUDA, and macOS Metal operation support.

Canonical contract: [Qwen3 dense model family](../architecture/inference/qwen3-model-family.md)

### A02 — Runtime precision and later quantization

The first correctness profile uses:

- Upstream unquantized Safetensors.
- FP16 runtime weights across the three required backends.
- FP16 wire activations.
- A 4,096-token context limit.
- Batch size 1.
- Non-thinking mode.

After the unquantized Qwen3-8B pipeline works, select exactly one 4-bit format. Do not begin with several quantization formats.

### A03 and A04 — Resolved

Control serialization, protocol versioning, errors, and activation framing are accepted in the [control protocol](../architecture/protocol/control-protocol.md) and [activation tensor frame](../architecture/protocol/activation-frame.md).

### A05 — Tokenizer and sampling ownership

Preferred rule:

- Coordinator owns the tokenizer.
- First stage owns embeddings.
- Final stage owns output head, sampling state, and next-token selection.

Define temperature, top-k, top-p, repetition penalties, random seeds, token history, and end-of-sequence handling.

### A06 — KV-cache contract

Define layout, data type, maximum context, batch allocation, grouped-query attention, sliding windows, cancellation, memory estimation, and eviction.

### A07 — Network benchmark and placement cost

Accepted in [Network benchmark and placement cost](../architecture/networking/network-benchmark.md) and [ADR-0012](../decisions/0012-network-benchmark-and-placement-cost.md).

### A08 — NAT and router integration

Accepted in [NAT and router mapping](../architecture/networking/nat-router-mapping.md) and [ADR-0013](../decisions/0013-nat-router-mapping-crates.md).

### A09 — Resolved

Invitation encoding and QUIC identity are accepted in the [Enrollment contract](../architecture/onboarding/enrollment-contract.md).

### A10 — Peer-record merge rules

Accepted in [Peer-record merge rules](../architecture/networking/peer-record-merge.md) and [ADR-0014](../decisions/0014-peer-record-merge-rules.md).

### A11 — Provider manifest generation

Confirm Hugging Face Hub as the first provider. Define immutable revision resolution, Safetensors header discovery, Model Family Adapter mapping, manifest cache key, and adapter-version behavior.

### A12 — Partial download validation

Define `Content-Range`, length, shape, data type, ETag, digest, incomplete-file, retry, and complete-shard fallback rules.

### A13 — Provider access and local cache

Provider credential persistence is accepted in [Persistent state](../architecture/system/persistent-state.md). Define provider-access capability reporting, disk reservation, cache limit, eviction thresholds, active-artifact protection, and incomplete-download cleanup.

## Implementation phases

### P01 — Workspace and native app shell

Build:

- Cargo workspace.
- `mesh-app` eframe executable named `mesh`.
- `mesh-node` runtime library.
- Typed GUI command and state channels.
- First-run and empty dashboard screens.

Proof:

> `cargo run --release` opens one native application without a frontend build or helper process on Windows, Linux, and macOS development hosts after platform prerequisites are installed.

### P02 — GUI-driven two-node enrollment

Build:

- Stable IDs and local persistence.
- Quinn endpoint.
- Invitation creation and input.
- `HELLO` and `WELCOME`.
- Peer Store.
- Enrollment progress screens.
- Reconnection after restart.

Proof:

> Windows, Linux, and macOS applications can enroll through the GUI, exchange static node state, restart, and reconnect without command-line arguments.

### P03 — Hardware and network discovery

Build:

- CPU, memory, and disk discovery.
- NVIDIA discovery through NVML on Windows and Linux.
- Apple Metal discovery on macOS.
- Capability reports.
- Directional network benchmark.
- GUI hardware and peer status.

Proof:

> Windows and Linux NVIDIA peers and a macOS Metal peer truthfully report measured hardware and network capabilities.

### P04 — Automatic direct connectivity

Build:

- IPv6 and IPv4 candidate collection.
- Automatic router mapping.
- Peer-assisted hole punching.
- Guided firewall and manual forwarding failures.

Proof:

> Enrollment uses automatic direct paths when available and gives one clear recovery action when unavailable.

### P05 — Resource reservations

Build:

- Resource offers.
- Expiring GPU, memory, disk, and execution leases.
- Commit and release.
- Concurrent coordinator conflicts.
- GUI reservation state.

Proof:

> Two coordinators cannot reserve the same local capacity.

### P06 — Model provider and cache

Build:

- `ModelProvider` interface.
- Hugging Face adapter.
- Immutable revision resolution.
- Local artifact cache.
- Safetensors metadata parser.
- Range downloads and complete-shard fallback.
- Parallel node preparation.
- GUI model selection, provider access, download progress, and failures.

Proof:

> Selected Windows, Linux, and macOS nodes automatically download different verified Qwen3-8B tensor assignments for one immutable revision.

### P07 — Single-node inference

Build:

- Dense Qwen3 Model Family Adapter.
- Complete `Qwen/Qwen3-4B` stage.
- Candle CUDA stage validated natively on Windows and Linux.
- Candle Metal stage validated on macOS Apple Silicon.
- Qwen3 tokenizer and non-thinking chat template.
- Qwen3 KV cache and seeded sampling.
- Streaming token output in the GUI.

Proof:

> The pinned Qwen3-4B model produces valid streamed output on Windows CUDA, Linux CUDA, and macOS Metal under the accepted correctness profile.

### P07.5 — Windows confidence and CI gate

Manually prove native Windows GUI, enrollment, model download, Qwen3-4B CUDA load, warm-up, and generation first.

After that implementation is stable enough to trust the build shape, add Windows CI for the cross-platform crates and native application. Add a CUDA build or GPU execution lane when an appropriate Windows runner is available.

Windows remains required before this gate. CI timing does not make the target optional.

### P08 — Replica inference

Build:

- Full-model replicas.
- Request routing.
- Dynamic batching.
- Per-node concurrency limits.
- Load and health reporting.

Proof:

> Independent Qwen3-4B requests run concurrently on separate complete-model nodes.

### P09 — Layer pipeline inference

Build:

- Layer placement.
- Partial stage loading.
- Activation wire format.
- Pipeline stage runtime.
- Final-stage sampling.
- Concurrent sequences.
- Cancellation, queue bounds, and backpressure.

Proof:

> The pinned Qwen3-8B model runs as continuous layer stages across at least two directly connected PCs, including a mixed Windows/Linux/macOS route.

### P10 — Failure and restart behavior

Build:

- Stage timeout and disconnect handling.
- Partial preparation rollback.
- Lease expiry.
- Request restart from prompt.
- Cache recovery after process restart.

Proof:

> A failed stage releases resources, preserves valid cache data, and leaves the deployment in a truthful state.

### P11 — Performance validation

Measure against one-node baselines:

- Qwen3-4B single-node latency and throughput on every required backend.
- Qwen3-4B replica throughput.
- Qwen3-8B two-stage and three-stage token delay.
- Qwen3-8B prompt-processing bandwidth.
- Windows-CUDA-to-Linux-CUDA, CUDA-to-Metal, and same-platform paths.
- Later quantization memory and speed.
- Model download, partial extraction, load, and warm-up time.

Only measured bottlenecks justify advanced optimization.

## Deferred work

- Distributed training and gradient synchronization.
- Live stage migration.
- Live KV-cache replication.
- General WAN tensor parallelism.
- Distributed mixture-of-experts routing.
- Speculative decoding.
- Replicated bottleneck stages.
- Peer-to-peer model cache sharing.
- Background system service and tray mode.
- Public relay support.

## Next decision

P05 is implemented. Keep Windows/macOS host proofs until the end. Before P06 model provider/cache work, lock A11–A13. Before P07 inference, also lock A05 and A06.

