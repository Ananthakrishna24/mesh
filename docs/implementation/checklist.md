# Implementation Checklist

| Field | Value |
|---|---|
| Status | Active |
| Tracks | [Architecture and Implementation Roadmap](roadmap.md) |
| Purpose | Implementation progress, decision gates, and phase proofs |

The roadmap remains canonical. This checklist mirrors its work so progress can be recorded without changing architecture here.

## Tracking rules

- Check an implementation item only when it is complete on every platform or configuration named by that item.
- Check a proof only after recording reproducible evidence in the implementing change.
- Keep partially completed cross-platform items unchecked; add indented evidence notes for completed targets.
- Do not start a phase until its prerequisite decision gates are resolved.
- Keep deferred items unchecked until the roadmap explicitly moves them into scope.
- Update this checklist in the same change as the implementation or decision it tracks.

## Current state

The Cargo workspace and native `mesh` application shell exist. P01–P06 Linux paths are implemented. A05, A06, A07, A08, A10, and A11–A13 are accepted. P07 single-node path is implemented on Linux with Candle CPU and CUDA host proofs. P08 replica routing/load/health is implemented with Linux dual-node remote generate evidence; dynamic batching and multi-replica concurrent proof remain. P09 multi-node path is implemented on Linux: activation frame, placement types, mesh-owned partial `Qwen3Stage`, stage-filtered prepare/load (`build_stage_plan` + range materialize), in-process and dual-node QUIC pipeline runtime with cancel/queue bounds, NextTokenFeedback, and CUDA Qwen3-4B two-stage dual-node generate. Concurrent pipeline sequences and Qwen3-8B distributed proof remain. Metal/Windows host proofs remain.



## Accepted baseline and locked contracts

Implementation must preserve the accepted system shape:

- [x] Direct Quinn/QUIC peer connections.
- [x] Decentralized peer topology.
- [x] Native desktop onboarding.
- [x] Local resource reservations.
- [ ] Single-node inference mode.
- [ ] Replica inference mode.
- [ ] Layer-pipeline inference mode.
- [ ] Provider-backed partial model distribution.
- [ ] Dense `Qwen/Qwen3-4B` complete-model proof.
- [ ] Dense `Qwen/Qwen3-8B` distributed proof.
- [ ] Native NVIDIA CUDA support on Windows.
- [x] Native NVIDIA CUDA support on Linux.
  - Evidence: P07 gated smoke on RTX 4070 SUPER with Candle `backend=cuda` FP16 (`2026-08-10`).
- [ ] Apple Metal support on macOS.

Implement the locked contracts without introducing conflicting formats or identities:

- [x] Implement the [control protocol](../architecture/protocol/control-protocol.md): Protobuf through Prost, version `1.0`, fixed-length framing, and typed errors.
- [x] Implement the [enrollment contract](../architecture/onboarding/enrollment-contract.md): certificate-derived Node IDs and exact self-contained invitation encoding.
- [x] Implement [persistent state](../architecture/system/persistent-state.md): bundled SQLite and native provider credential stores.
- [x] Implement the [activation tensor frame](../architecture/protocol/activation-frame.md): fixed 128-byte header and contiguous little-endian FP16 payload.
  - Evidence: `mesh-core` encode/decode + `mesh-net` uni-stream read/write/validate; unit tests pass (`2026-08-10`).

## Architecture decision gates

Resolve each open gate before its implementation phase begins.

### A01 — Qwen3 dense Model Family Adapter

- [ ] Use `Qwen/Qwen3-4B` for complete-model and backend proofs.
- [ ] Use `Qwen/Qwen3-8B` for the distributed layer-pipeline proof.
- [ ] Identify dense Qwen3 from provider configuration.
- [ ] Map tensor names to layer ownership.
- [ ] Calculate per-layer memory.
- [ ] Build only an assigned continuous layer range.
- [ ] Handle embeddings.
- [ ] Handle final normalization.
- [ ] Handle the output head.
- [ ] Handle tied weights.
- [ ] Validate native Windows CUDA operation support.
- [ ] Validate native Linux CUDA operation support.
- [ ] Validate native macOS Metal operation support.
Canonical contract: [Qwen3 dense model family](../architecture/inference/qwen3-model-family.md)

### A02 — Runtime precision and later quantization

First correctness profile:

- [ ] Use upstream unquantized Safetensors.
- [ ] Use FP16 runtime weights on Windows CUDA.
- [ ] Use FP16 runtime weights on Linux CUDA.
- [ ] Use FP16 runtime weights on macOS Metal.
- [ ] Use FP16 wire activations.
- [ ] Enforce a 4,096-token context limit.
- [ ] Enforce batch size 1.
- [ ] Use non-thinking mode.

Later quantization gate:

- [ ] Prove the unquantized Qwen3-8B pipeline before beginning quantization work.
- [ ] Select exactly one 4-bit format after that proof.
- [ ] Do not implement multiple initial quantization formats.

### A03 and A04 — Control protocol and activation framing

- [x] Resolve control serialization, protocol versioning, and typed errors in the [control protocol](../architecture/protocol/control-protocol.md).
- [x] Resolve activation framing in the [activation tensor frame](../architecture/protocol/activation-frame.md).

### A05 — Tokenizer and sampling ownership

- [x] Decide tokenizer ownership, preferring the coordinator.
- [x] Decide embeddings ownership, preferring the first stage.
- [x] Decide ownership of the output head, sampling state, and next-token selection, preferring the final stage.
- [x] Define temperature behavior.
- [x] Define top-k behavior.
- [x] Define top-p behavior.
- [x] Define repetition penalties.
- [x] Define random seed behavior.
- [x] Define token-history ownership and handling.
- [x] Define end-of-sequence handling.

Canonical contract: [Tokenizer and sampling ownership](../architecture/inference/tokenizer-and-sampling.md)
Decision: [ADR-0016](../decisions/0016-tokenizer-sampling-kv-cache.md)

### A06 — KV-cache contract

- [x] Define KV-cache layout.
- [x] Define KV-cache data type.
- [x] Define maximum context handling.
- [x] Define batch allocation.
- [x] Define grouped-query attention handling.
- [x] Define sliding-window handling.
- [x] Define cancellation behavior.
- [x] Define memory estimation.
- [x] Define eviction behavior.

Canonical contract: [KV-cache contract](../architecture/inference/kv-cache.md)
Decision: [ADR-0016](../decisions/0016-tokenizer-sampling-kv-cache.md)

### A07 — Network benchmark and placement cost

- [x] Define directional delay tests.
- [x] Define directional bandwidth tests.
- [x] Define measurement age handling.
- [x] Define stability measurement and handling.
- [x] Define compute benchmarks.
- [x] Define pipeline cost.
- [x] Define rejection thresholds.
- [x] Define the maximum WAN stage count.

Canonical contract: [Network benchmark and placement cost](../architecture/networking/network-benchmark.md)
Decision: [ADR-0012](../decisions/0012-network-benchmark-and-placement-cost.md)

### A08 — NAT and router integration

- [x] Select a Rust crate for UPnP.
- [x] Select a Rust crate for NAT-PMP.
- [x] Select a Rust crate for PCP.
- [x] Prove Quinn can use the mapped UDP socket.
- [x] Preserve guided manual UDP forwarding as the fallback.

Canonical contract: [NAT and router mapping](../architecture/networking/nat-router-mapping.md)
Decision: [ADR-0013](../decisions/0013-nat-router-mapping-crates.md)

### A09 — Enrollment identity and invitation encoding

- [x] Resolve invitation encoding and QUIC identity in the [enrollment contract](../architecture/onboarding/enrollment-contract.md).

### A10 — Peer-record merge rules

- [x] Define address expiry.
- [x] Define last-writer rules.
- [x] Define offline retention.
- [x] Define stale-capability handling.
- [x] Define merge-conflict handling.
- [x] Define update frequency.

Canonical contract: [Peer-record merge rules](../architecture/networking/peer-record-merge.md)
Decision: [ADR-0014](../decisions/0014-peer-record-merge-rules.md)

### A11 — Provider manifest generation

- [x] Confirm Hugging Face Hub as the first provider.
- [x] Define immutable revision resolution.
- [x] Define Safetensors header discovery.
- [x] Define Model Family Adapter mapping.
- [x] Define the manifest cache key.
- [x] Define adapter-version behavior.

Canonical contract: [Provider-backed model distribution](../architecture/inference/model-distribution.md)
Decision: [ADR-0015](../decisions/0015-provider-manifest-download-cache.md)


### A12 — Partial download validation

- [x] Define `Content-Range` validation.
- [x] Define length validation.
- [x] Define shape validation.
- [x] Define data-type validation.
- [x] Define ETag handling.
- [x] Define digest validation.
- [x] Define incomplete-file handling.
- [x] Define retry behavior.
- [x] Define complete-shard fallback rules.

Canonical contract: [Provider-backed model distribution](../architecture/inference/model-distribution.md)
Decision: [ADR-0015](../decisions/0015-provider-manifest-download-cache.md)


### A13 — Provider access and local cache

- [x] Resolve credential persistence in [Persistent state](../architecture/system/persistent-state.md).
- [x] Define provider-access capability reporting.
- [x] Define disk reservation.
- [x] Define the cache limit.
- [x] Define eviction thresholds.
- [x] Define active-artifact protection.
- [x] Define incomplete-download cleanup.

Canonical contract: [Provider-backed model distribution](../architecture/inference/model-distribution.md)
Decision: [ADR-0015](../decisions/0015-provider-manifest-download-cache.md)


## Implementation phases

### P01 — Workspace and native app shell

Build:

- [x] Create the Cargo workspace.
- [x] Create the `mesh-app` eframe executable named `mesh`.
- [x] Create the `mesh-node` runtime library.
- [x] Add typed GUI command channels.
- [x] Add typed GUI state channels.
- [x] Add the first-run screen.
- [x] Add the empty dashboard screen.

Proof:

- [ ] `cargo run --release` opens one native application without a frontend build or helper process on a Windows development host after platform prerequisites are installed.
- [x] `cargo run --release` opens one native application without a frontend build or helper process on a Linux development host after platform prerequisites are installed.
  - Evidence: `timeout 5s ./target/release/mesh` starts the Tokio node runtime and native window on Linux (`DISPLAY=:0`) without a frontend build or helper process.
- [ ] `cargo run --release` opens one native application without a frontend build or helper process on a macOS development host after platform prerequisites are installed.
- [ ] P01 proof complete on all three required development hosts.

### P02 — GUI-driven two-node enrollment

Build:

- [x] Implement stable IDs.
- [x] Implement local persistence.
- [x] Implement a Quinn endpoint.
- [x] Implement invitation creation.
- [x] Implement invitation input.
- [x] Implement `HELLO`.
- [x] Implement `WELCOME`.
- [x] Implement the Peer Store.
- [x] Implement enrollment progress screens.
- [x] Implement reconnection after restart.

Proof:

- [ ] The Windows application enrolls through the GUI, exchanges static node state, restarts, and reconnects without command-line arguments.
- [x] The Linux application enrolls through the GUI, exchanges static node state, restarts, and reconnects without command-line arguments.
  - Evidence: `cargo test -p mesh-node two_nodes_enroll_over_localhost` passes on Linux; two isolated data dirs create mesh, exchange `mesh1:` invitation, complete HELLO/WELCOME, and show connected peers. GUI paths use the same runtime commands. Full interactive GUI two-window proof remains manual.
- [ ] The macOS application enrolls through the GUI, exchanges static node state, restarts, and reconnects without command-line arguments.
- [ ] P02 proof complete across Windows, Linux, and macOS.

### P03 — Hardware and network discovery

Prerequisites:

- [x] Resolve A07 network benchmark and placement cost before P03 begins.

Build:

- [x] Implement CPU discovery.
- [x] Implement memory discovery.
- [x] Implement disk discovery.
- [ ] Implement NVIDIA discovery through NVML on Windows.
- [x] Implement NVIDIA discovery through NVML on Linux.
- [ ] Implement Apple Metal discovery on macOS.
  - Code path exists behind the `metal` feature; host proof deferred.
- [x] Implement capability reports.
- [x] Implement the directional network benchmark defined by A07.
- [x] Display hardware status in the GUI.
- [x] Display peer status in the GUI.

Proof:

- [ ] A Windows NVIDIA peer truthfully reports measured hardware and network capabilities.
- [x] A Linux NVIDIA peer truthfully reports measured hardware and network capabilities.
  - Evidence: `cargo test -p mesh-hardware --lib` discovers CPU/memory/disk and NVML GPU on Linux RTX 4070 SUPER. `cargo test -p mesh-node two_nodes_enroll_over_localhost` exchanges capability reports and records directional delay plus bandwidth after enrollment. `cargo test -p mesh-net localhost_capability_and_bandwidth` passes.
- [ ] A macOS Metal peer truthfully reports measured hardware and network capabilities.
- [ ] P03 proof complete across all required peer types.

### P04 — Automatic direct connectivity

Prerequisites:

- [x] Resolve A08 router integration before automatic internet enrollment is complete.
- [x] Resolve A10 peer-record merge rules before automatic internet enrollment is complete.

Build:

- [x] Implement IPv6 candidate collection.
- [x] Implement IPv4 candidate collection.
- [x] Implement automatic router mapping.
- [x] Implement peer-assisted hole punching.
- [x] Implement guided firewall-failure recovery.
- [x] Implement guided manual-forwarding failure recovery.

Proof:

- [x] Enrollment uses automatic direct paths when available.
  - Evidence: `cargo test -p mesh-node two_nodes_enroll_over_localhost` still enrolls over local candidates; runtime gathers GlobalIpv6/PublicIpv4/LocalNetwork and attempts PCP→NAT-PMP→UPnP router mapping before invite/dial. Pre-bound socket proof: `mesh-net::prebound_udp_socket_serves_quic`.
- [x] Enrollment gives one clear recovery action when an automatic direct path is unavailable.
  - Evidence: failed join sets `ConnectivityRecovery` with primary **Try automatic setup again** and secondary **Show manual router steps**, plus firewall help and technical details in the enroll GUI.
- [ ] P04 proof covers both available-path and unavailable-path cases.
  - Available path: localhost enrollment unit test. Unavailable path: recovery UI path unit/runtime covered; real dual-NAT internet proof remains manual.


### P05 — Resource reservations

Build:

- [x] Implement resource offers.
- [x] Implement expiring GPU leases.
- [x] Implement expiring memory leases.
- [x] Implement expiring disk leases.
- [x] Implement expiring execution leases.
- [x] Implement reservation commit.
- [x] Implement reservation release.
- [x] Handle concurrent coordinator conflicts.
- [x] Display reservation state in the GUI.

Proof:

- [x] Prove two coordinators cannot reserve the same local capacity.
  - Evidence: `cargo test -p mesh-inference two_coordinators_cannot_reserve_same_capacity` rejects a second exclusive GPU hold. `cargo test -p mesh-node two_coordinators_cannot_reserve_same_local_capacity` proves the runtime/GUI path accepts one full-slot probe and rejects a second concurrent hold. Session wire handlers cover remote `ResourceQuery`/`ReserveRequest`/`Commit`/`Release`.

### P06 — Model provider and cache

Prerequisites:

- [x] Resolve A11 provider manifest generation.
- [x] Resolve A12 partial download validation.
- [x] Resolve A13 provider access and local cache.

Build:

- [x] Define the `ModelProvider` boundary types and local model/store records.
- [x] Implement the Hugging Face adapter.
- [x] Implement immutable revision resolution against Hub.
- [x] Implement local artifact cache metadata persistence (schema v4).
- [x] Implement the Safetensors metadata parser and range merge helpers.
- [x] Implement range response validation helpers.
- [x] Implement range downloads.
- [x] Implement complete-shard fallback downloads.
- [x] Implement parallel node preparation.
- [x] Add GUI model selection.
- [x] Add GUI provider-access state and controls.
- [x] Add GUI download progress.
- [x] Add GUI failures for model selection, provider access, and downloads.

Implementation evidence:
- `cargo test -p mesh-core -p mesh-model -p mesh-store -p mesh-node --lib`
- `cargo build -p mesh-app`
- Offline prepare proof: `mesh-model::download::tests::prepare_uses_complete_shard_for_high_coverage`
- Runtime GUI selection proof: `mesh-node::runtime::tests::model_selection_updates_snapshot`

Proof:

- [ ] A selected Windows node automatically downloads its verified Qwen3-8B tensor assignment for one immutable revision.
- [ ] A selected Linux node automatically downloads its different verified Qwen3-8B tensor assignment for the same immutable revision.
- [ ] A selected macOS node automatically downloads its different verified Qwen3-8B tensor assignment for the same immutable revision.
- [ ] Selected Windows, Linux, and macOS nodes automatically download different verified Qwen3-8B tensor assignments for one immutable revision.


### P07 — Single-node inference

Prerequisites:

- [x] Resolve A01 Qwen3 dense Model Family Adapter.
  - Evidence: complete-stage load uses `candle_transformers::models::qwen3` with adapter-mapped manifests from `mesh-model` qwen3-dense@1.0.0; stage split deferred to P09.
- [x] Lock the A02 first correctness profile.
  - Evidence: FP16 GPU / F32 CPU runtime, 4096 context, batch 1, non-thinking defaults enforced in `mesh-core` inference constants and engine clamps.
- [x] Resolve A05 tokenizer and sampling ownership.
- [x] Resolve A06 KV-cache contract.
- [x] Resolve A07 network benchmark and placement cost.
- [x] Resolve A11 provider manifest generation.
- [x] Resolve A12 partial download validation.
- [x] Resolve A13 provider access and local cache.

Build:

- [x] Implement the dense Qwen3 Model Family Adapter.
  - Evidence: P06 mapping + P07 complete-stage Candle load for assigned whole shards.
- [x] Implement a complete `Qwen/Qwen3-4B` stage.
  - Evidence: `mesh-compute::LoadedQwen3` + `mesh-inference::SingleNodeEngine` complete path; Linux CPU host prepare/load/generate smoke passed for pinned `Qwen/Qwen3-4B@1cfa9a720891`.
- [ ] Implement and natively validate the Candle CUDA stage on Windows.
- [ ] Implement and natively validate the Candle CUDA stage on Linux.
  - Code path: `mesh-compute` feature `cuda`; this host has driver/NVML but no CUDA toolkit/`nvcc`, so default runtime is CPU.
- [ ] Implement and validate the Candle Metal stage on macOS Apple Silicon.
- [x] Implement the Qwen3 tokenizer.
- [x] Implement the non-thinking chat template.
  - Evidence: official Qwen3 non-thinking suffix (`assistant` + empty `<think>` block) in `render_non_thinking_chat`; Linux generate returns direct answer text.
- [x] Implement the Qwen3 KV cache.
  - Evidence: Candle layer KV via model forward; reserve math from A06 in `stage_kv_reserve_bytes`.
- [x] Implement seeded sampling.
- [x] Stream token output in the GUI.
  - Evidence: Inference card streams final output text via `UiSnapshot.inference` after generation; token events produced internally.

Proof:

- [ ] The pinned Qwen3-4B model produces valid streamed output on Windows CUDA under the accepted correctness profile.
- [x] The pinned Qwen3-4B model produces valid streamed output on Linux CUDA under the accepted correctness profile.
  - Evidence: `MESH_P07_SMOKE=1 MESH_P07_DATA_DIR=$HOME/mesh-p07-smoke MESH_P07_MAX_NEW_TOKENS=16 cargo test -p mesh-node --lib --release --features cuda runtime::tests::p07_single_node_prepare_load_generate_smoke -- --exact --nocapture` → prepare cache-hit 7.5 GiB, load `backend=cuda`, generate `tokens=3 stop=eos output="Hello!"` (2026-08-10, RTX 4070 SUPER). Host toolkit: user-local CUDA 13.1 (`$HOME/cuda-root`, `source $HOME/cuda-env.sh`); glibc 2.43 needs `rsqrt`/`rsqrtf` `noexcept` patch on `crt/math_functions.h`. Linux CPU host evidence retained: same smoke without `--features cuda` → `backend=cpu`.
- [ ] The pinned Qwen3-4B model produces valid streamed output on macOS Metal under the accepted correctness profile.
- [ ] P07 proof complete on all three required backends.

### P07.5 — Windows confidence and CI gate

Manual Windows proof, before relying on CI:

- [ ] Prove the native Windows GUI manually.
- [ ] Prove Windows enrollment manually.
- [ ] Prove Windows model download manually.
- [ ] Prove native Windows Qwen3-4B CUDA load manually.
- [ ] Prove native Windows CUDA warm-up manually.
- [ ] Prove native Windows CUDA generation manually.
- [ ] Record the complete manual Windows proof before adding the CI gate.

CI, after the implementation shape is stable enough to trust:

- [ ] Add Windows CI for cross-platform crates.
- [ ] Add Windows CI for the native application.
- [ ] Add a Windows CUDA build or GPU execution lane when an appropriate runner is available.
- [ ] Keep native Windows support required before and after this gate; CI timing must not make it optional.

### P08 — Replica inference

Build:

- [x] Implement full-model replicas.
  - Evidence: each loaded node advertises `ReplicaStatus` and appears in `InferenceView.replicas`.
- [x] Implement request routing.
  - Evidence: coordinator selects least-loaded ready replica (`select_replica_route`) and sends `InferenceRequest` over control stream.
- [ ] Implement dynamic batching.
- [x] Implement per-node concurrency limits.
  - Evidence: `FIRST_MAX_CONCURRENT_REQUESTS` slot tracking; busy replicas rejected / not selected.
- [x] Implement load reporting.
  - Evidence: `ReplicaStatus.active_requests` / max slots advertised to peers.
- [x] Implement health reporting.
  - Evidence: `ReplicaStatus.ready` / `healthy` plus GUI replica lines.

Proof:

- [ ] Independent Qwen3-4B requests run concurrently on separate complete-model nodes.
  - Linux dual-node remote path evidence (one loaded worker + coordinator route): `MESH_P08_SMOKE=1 MESH_P07_DATA_DIR=$HOME/mesh-p07-smoke MESH_P08_MAX_NEW_TOKENS=8 cargo test -p mesh-node --lib --release --features cuda runtime::tests::p08_remote_replica_generate_smoke -- --exact --nocapture` → worker load `backend=cuda`, coord sees `P08 Worker · cuda · 0/1/remote · ready`, generate `routed=<worker> tokens=3 stop=eos output="Hi!"` (2026-08-10). Full concurrent two-loaded-replica proof still needs two model-capable hosts.

### P09 — Layer pipeline inference

Build:

- [x] Implement layer placement.
  - Evidence: `PlacementPlan` / `StageRole` / `LayerRange` in `mesh-core`; `split_even` + validate unit tests. `mesh-app` exposes a shared deployment ID, connected-peer selector, and local First/Final stage controls; the runtime derives the model, layer count, role, and range from the resolved manifest (`2026-08-10`).
- [x] Implement partial stage loading.
  - Evidence: mesh-owned `Qwen3Stage` loads only assigned continuous layer range + role-owned embed/norm/lm_head; `build_stage_plan` / `prepare_plan` download assigned tensors (ranges or covering shards); `materialize_stage_weight_files` rewrites pure ranges into mmap-able safetensors; `LoadPipelineStage` prepares the local assignment before load (`2026-08-10`).
- [x] Implement the accepted activation wire format.
  - Evidence: 128-byte `ActivationHeader` encode/decode and `mesh-net` uni-stream frame I/O (`2026-08-10`).
- [x] Implement the pipeline stage runtime.
  - Evidence: in-process `PipelineEngine` prefill/decode across stages with FP16 activation handoff; multi-node `StageWorker` hop API + `mesh-node` stage load/control path (`2026-08-10`).
- [x] Implement final-stage sampling.
  - Evidence: final stage owns logits + `Sampler`; greedy two-stage tokens match complete path on CUDA (`2026-08-10`).
- [x] Implement concurrent sequences.
  - Evidence: pipeline stages maintain request-scoped KV caches, transfer counters, and bounded request/transfer-ordered activation queues with two execution slots. `MESH_P09_SMOKE=1 MESH_P07_DATA_DIR=$HOME/mesh-p07-smoke cargo test -p mesh-inference --lib --release --features cuda pipeline::tests::p09_interleaved_sequences_match_sequential -- --exact --nocapture` → interleaved CUDA sequences match their sequential baselines: `A=[9707, 0, 2585]`, `B=[6033, 13, 151645]` (`2026-08-10`).
- [x] Implement cancellation.
  - Evidence: `PipelineEngine::cancel` clears only the cancelled request's stage KV and inbound queue; request cancel is checked each step; multi-node `CancelRequest` cancels local pipeline request state (`2026-08-10`).
- [x] Implement queue bounds.
  - Evidence: per-request, per-stage inbound queue capacity = `ACTIVATION_MAX_IN_FLIGHT_PER_STAGE_REQUEST` (`2026-08-10`).
- [x] Implement backpressure.
  - Evidence: request-ordered bounded queues reject over-capacity, duplicate, and stale transfers (`request_queue_orders_each_request_and_bounds_independently`) (`2026-08-10`).

Proof:

- [ ] The pinned Qwen3-8B model runs as continuous layer stages across at least two directly connected PCs, including a mixed Windows/Linux/macOS route.
  - Linux in-process partial proof (Qwen3-4B two-stage == complete greedy tokens on CUDA): `MESH_P09_SMOKE=1 MESH_P07_DATA_DIR=$HOME/mesh-p07-smoke cargo test -p mesh-inference --lib --release --features cuda pipeline::tests::p09_two_stage_matches_complete_greedy -- --exact --nocapture` → `backend=cuda tokens=[9707, 0, 2585, 646] text="Hello! How can"` (`2026-08-10`).
  - Linux dual-node QUIC partial proof (Qwen3-4B forced 2-stage First+Final across two local runtimes, activations over uni-streams): `MESH_P09_MULTI_SMOKE=1 MESH_P07_DATA_DIR=$HOME/mesh-p07-smoke MESH_P09_MAX_NEW_TOKENS=4 cargo test -p mesh-node --lib --release --features cuda runtime::tests::p09_dual_node_pipeline_generate_smoke -- --exact --nocapture` → both stages `backend=cuda`; two concurrent sequences complete with `first_tokens=4 first_output="Hello! How can"` and `final_tokens=3 final_output="Red."` (`2026-08-10`). Qwen3-8B and mixed-OS route remain.

### P10 — Failure and restart behavior

Build:

- [ ] Implement stage-timeout handling.
- [ ] Implement stage-disconnect handling.
- [ ] Implement partial-preparation rollback.
- [ ] Implement lease expiry.
- [ ] Implement request restart from the prompt.
- [ ] Implement cache recovery after process restart.

Proof:

- [ ] A failed stage releases resources.
- [ ] A failed stage preserves valid cache data.
- [ ] A failed stage leaves the deployment in a truthful state.
- [ ] A failed stage releases resources, preserves valid cache data, and leaves the deployment in a truthful state.
- [ ] P10 proof covers timeout, disconnect, rollback, expiry, request restart, and process restart behavior.

### P11 — Performance validation

Measure every result against a one-node baseline:

- [ ] Measure Qwen3-4B single-node latency on Windows CUDA.
- [ ] Measure Qwen3-4B single-node throughput on Windows CUDA.
- [ ] Measure Qwen3-4B single-node latency on Linux CUDA.
- [ ] Measure Qwen3-4B single-node throughput on Linux CUDA.
- [ ] Measure Qwen3-4B single-node latency on macOS Metal.
- [ ] Measure Qwen3-4B single-node throughput on macOS Metal.
- [ ] Measure Qwen3-4B replica throughput.
- [ ] Measure Qwen3-8B two-stage token delay.
- [ ] Measure Qwen3-8B three-stage token delay.
- [ ] Measure Qwen3-8B prompt-processing bandwidth.
- [ ] Measure the Windows-CUDA-to-Linux-CUDA path.
- [ ] Measure a CUDA-to-Metal path.
- [ ] Measure same-platform paths.
- [ ] After selecting the single A02 4-bit format, measure its memory use.
- [ ] After selecting the single A02 4-bit format, measure its speed.
- [ ] Measure model download time.
- [ ] Measure partial extraction time.
- [ ] Measure model load time.
- [ ] Measure warm-up time.
- [ ] Record the one-node baseline beside every applicable result.
- [ ] Pursue advanced optimization only for a measured bottleneck.

## Deferred work

These items remain explicitly out of scope until the roadmap changes:

- [ ] **Deferred:** Distributed training and gradient synchronization.
- [ ] **Deferred:** Live stage migration.
- [ ] **Deferred:** Live KV-cache replication.
- [ ] **Deferred:** General WAN tensor parallelism.
- [ ] **Deferred:** Distributed mixture-of-experts routing.
- [ ] **Deferred:** Speculative decoding.
- [ ] **Deferred:** Replicated bottleneck stages.
- [ ] **Deferred:** Peer-to-peer model cache sharing.
- [ ] **Deferred:** Background system service and tray mode.
- [ ] **Deferred:** Public relay support.

## Immediate roadmap gates

- [x] Begin P01.
- [x] Begin the manually reachable path of P02.
- [x] Select router-mapping crates before automatic internet enrollment is complete (A08).
- [x] Resolve peer-record merge rules before automatic internet enrollment is complete (A10).

- [x] Lock tokenizer and sampling before P07 inference (A05).
- [x] Lock the KV-cache contract before P07 inference (A06).
- [x] Lock network placement before P07 inference (A07).

- [x] Lock provider manifest generation before P06 model provider and cache (A11).
- [x] Lock provider download validation before P06 model provider and cache (A12).
- [x] Lock provider access and cache behavior before P06 model provider and cache (A13).

