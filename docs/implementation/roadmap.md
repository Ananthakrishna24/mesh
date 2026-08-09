# Architecture and Implementation Roadmap

| Field | Value |
|---|---|
| Status | Active |
| Canonical for | Remaining decisions, implementation order, and phase proofs |
| Parent | [Documentation index](../README.md) |

## Current state

The system shape is accepted and documented. No Rust workspace or executable exists yet.

Accepted areas:

- Direct Quinn/QUIC peer connections.
- Decentralized peer topology.
- Native desktop onboarding.
- Local resource reservations.
- Single-node, replica, and layer-pipeline inference modes.
- Provider-backed partial model distribution.
- Native CUDA and Metal backend boundaries.

## Remaining architecture decisions

Resolve these before their implementation phase begins.

### A01 — First Model Family Adapter

Select one dense decoder model family. The adapter must:

- Identify the architecture from provider configuration.
- Map tensor names to layer ownership.
- Calculate per-layer memory.
- Build only an assigned continuous layer range.
- Handle embedding, final normalization, output head, and tied weights.
- Validate CUDA and Metal operation support.

Recommended first direction: a small dense Llama-compatible model for correctness, followed by a larger model using the same adapter.

### A02 — First weight format and quantization

Select:

- FP16 correctness baseline.
- One later 4-bit format for large-model capacity.
- Exact CUDA and Metal support requirements.

Do not begin with several quantization formats.

### A03 — Protocol serialization and versioning

Define exact binary schemas, compatibility rules, timeouts, and error codes for:

- Mesh handshake and peer updates.
- Resource offers and leases.
- Model preparation and readiness.
- Inference requests and cancellation.
- Activation and token transfer.

### A04 — Activation wire format

Define deployment and request IDs, stage index, sequence position, tensor shape, data type, byte order, layout, payload framing, limits, and optional checks.

Initial direction: contiguous little-endian FP16 data with one logical activation per QUIC transfer stream.

### A05 — Tokenizer and sampling ownership

Preferred rule:

- Coordinator owns the tokenizer.
- First stage owns embeddings.
- Final stage owns output head, sampling state, and next-token selection.

Define temperature, top-k, top-p, repetition penalties, random seeds, token history, and end-of-sequence handling.

### A06 — KV-cache contract

Define layout, data type, maximum context, batch allocation, grouped-query attention, sliding windows, cancellation, memory estimation, and eviction.

### A07 — Network benchmark and placement cost

Define directional delay and bandwidth tests, measurement age, stability, compute benchmarks, pipeline cost, rejection thresholds, and maximum WAN stage count.

### A08 — NAT and router integration

Select Rust crates for UPnP, NAT-PMP, and PCP. Prove Quinn can use the mapped UDP socket. Keep manual UDP forwarding as the guided fallback.

### A09 — Invite encoding and QUIC identity

Finalize invitation text, file, URI, expiry, certificate creation, certificate acceptance, stable Node ID binding, and restart persistence.

Canonical user contract: [Enrollment contract](../architecture/onboarding/enrollment-contract.md)

### A10 — Peer-record merge rules

Define address expiry, last-writer rules, offline retention, stale capability handling, merge conflicts, and update frequency.

### A11 — Provider manifest generation

Confirm Hugging Face Hub as the first provider. Define immutable revision resolution, Safetensors header discovery, Model Family Adapter mapping, manifest cache key, and adapter-version behavior.

### A12 — Partial download validation

Define `Content-Range`, length, shape, data type, ETag, digest, incomplete-file, retry, and complete-shard fallback rules.

### A13 — Provider access and local cache

Define local credential storage, provider-access capability, disk reservation, cache limit, eviction, active-artifact protection, and incomplete-download cleanup.

## Implementation phases

### P01 — Workspace and native app shell

Build:

- Cargo workspace.
- `mesh-app` eframe executable named `mesh`.
- `mesh-node` runtime library.
- Typed GUI command and state channels.
- First-run and empty dashboard screens.

Proof:

> `cargo run --release` opens one native application without a frontend build or helper process.

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

> Two internet-connected PCs enroll through the GUI, exchange static node state, restart, and reconnect without command-line arguments.

### P03 — Hardware and network discovery

Build:

- CPU, memory, and disk discovery.
- NVIDIA discovery through NVML.
- Apple Metal discovery.
- Capability reports.
- Directional network benchmark.
- GUI hardware and peer status.

Proof:

> Each enrolled PC shows its own and connected peers' measured capabilities.

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

> Selected nodes automatically download different verified tensor assignments for one immutable model revision.

### P07 — Single-node inference

Build:

- First Model Family Adapter.
- Candle CUDA stage.
- Candle Metal stage.
- Tokenizer.
- KV cache.
- Sampling.
- Streaming token output in the GUI.

Proof:

> The same accepted model produces valid streamed output on one supported CUDA or Metal node.

### P08 — Replica inference

Build:

- Full-model replicas.
- Request routing.
- Dynamic batching.
- Per-node concurrency limits.
- Load and health reporting.

Proof:

> Independent requests run concurrently on separate complete-model nodes.

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

> A model that does not fit on one selected GPU runs across at least two directly connected PCs.

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

- Replica throughput.
- Two-stage and three-stage token delay.
- Prompt-processing bandwidth.
- CUDA-to-CUDA and CUDA-to-Metal paths.
- Quantization memory and speed.
- Model download and warm-up time.

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

Select the first model family, model size, and correctness format. This fixes the first Model Family Adapter, tensor mapping, KV-cache shape, required CUDA and Metal operations, and the end-to-end inference proof.
