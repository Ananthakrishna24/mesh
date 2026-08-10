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

The Cargo workspace and native `mesh` application shell exist. P01 is implemented on Linux; Windows and macOS host proofs remain.

## Accepted baseline and locked contracts

Implementation must preserve the accepted system shape:

- [ ] Direct Quinn/QUIC peer connections.
- [ ] Decentralized peer topology.
- [ ] Native desktop onboarding.
- [ ] Local resource reservations.
- [ ] Single-node inference mode.
- [ ] Replica inference mode.
- [ ] Layer-pipeline inference mode.
- [ ] Provider-backed partial model distribution.
- [ ] Dense `Qwen/Qwen3-4B` complete-model proof.
- [ ] Dense `Qwen/Qwen3-8B` distributed proof.
- [ ] Native NVIDIA CUDA support on Windows.
- [ ] Native NVIDIA CUDA support on Linux.
- [ ] Apple Metal support on macOS.

Implement the locked contracts without introducing conflicting formats or identities:

- [ ] Implement the [control protocol](../architecture/protocol/control-protocol.md): Protobuf through Prost, version `1.0`, fixed-length framing, and typed errors.
- [ ] Implement the [enrollment contract](../architecture/onboarding/enrollment-contract.md): certificate-derived Node IDs and exact self-contained invitation encoding.
- [ ] Implement [persistent state](../architecture/system/persistent-state.md): bundled SQLite and native provider credential stores.
- [ ] Implement the [activation tensor frame](../architecture/protocol/activation-frame.md): fixed 128-byte header and contiguous little-endian FP16 payload.

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

- [ ] Decide tokenizer ownership, preferring the coordinator.
- [ ] Decide embeddings ownership, preferring the first stage.
- [ ] Decide ownership of the output head, sampling state, and next-token selection, preferring the final stage.
- [ ] Define temperature behavior.
- [ ] Define top-k behavior.
- [ ] Define top-p behavior.
- [ ] Define repetition penalties.
- [ ] Define random seed behavior.
- [ ] Define token-history ownership and handling.
- [ ] Define end-of-sequence handling.

### A06 — KV-cache contract

- [ ] Define KV-cache layout.
- [ ] Define KV-cache data type.
- [ ] Define maximum context handling.
- [ ] Define batch allocation.
- [ ] Define grouped-query attention handling.
- [ ] Define sliding-window handling.
- [ ] Define cancellation behavior.
- [ ] Define memory estimation.
- [ ] Define eviction behavior.

### A07 — Network benchmark and placement cost

- [ ] Define directional delay tests.
- [ ] Define directional bandwidth tests.
- [ ] Define measurement age handling.
- [ ] Define stability measurement and handling.
- [ ] Define compute benchmarks.
- [ ] Define pipeline cost.
- [ ] Define rejection thresholds.
- [ ] Define the maximum WAN stage count.

### A08 — NAT and router integration

- [ ] Select a Rust crate for UPnP.
- [ ] Select a Rust crate for NAT-PMP.
- [ ] Select a Rust crate for PCP.
- [ ] Prove Quinn can use the mapped UDP socket.
- [ ] Preserve guided manual UDP forwarding as the fallback.

### A09 — Enrollment identity and invitation encoding

- [x] Resolve invitation encoding and QUIC identity in the [enrollment contract](../architecture/onboarding/enrollment-contract.md).

### A10 — Peer-record merge rules

- [ ] Define address expiry.
- [ ] Define last-writer rules.
- [ ] Define offline retention.
- [ ] Define stale-capability handling.
- [ ] Define merge-conflict handling.
- [ ] Define update frequency.

### A11 — Provider manifest generation

- [ ] Confirm Hugging Face Hub as the first provider.
- [ ] Define immutable revision resolution.
- [ ] Define Safetensors header discovery.
- [ ] Define Model Family Adapter mapping.
- [ ] Define the manifest cache key.
- [ ] Define adapter-version behavior.

### A12 — Partial download validation

- [ ] Define `Content-Range` validation.
- [ ] Define length validation.
- [ ] Define shape validation.
- [ ] Define data-type validation.
- [ ] Define ETag handling.
- [ ] Define digest validation.
- [ ] Define incomplete-file handling.
- [ ] Define retry behavior.
- [ ] Define complete-shard fallback rules.

### A13 — Provider access and local cache

- [x] Resolve credential persistence in [Persistent state](../architecture/system/persistent-state.md).
- [ ] Define provider-access capability reporting.
- [ ] Define disk reservation.
- [ ] Define the cache limit.
- [ ] Define eviction thresholds.
- [ ] Define active-artifact protection.
- [ ] Define incomplete-download cleanup.

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

- [ ] Implement stable IDs.
- [ ] Implement local persistence.
- [ ] Implement a Quinn endpoint.
- [ ] Implement invitation creation.
- [ ] Implement invitation input.
- [ ] Implement `HELLO`.
- [ ] Implement `WELCOME`.
- [ ] Implement the Peer Store.
- [ ] Implement enrollment progress screens.
- [ ] Implement reconnection after restart.

Proof:

- [ ] The Windows application enrolls through the GUI, exchanges static node state, restarts, and reconnects without command-line arguments.
- [ ] The Linux application enrolls through the GUI, exchanges static node state, restarts, and reconnects without command-line arguments.
- [ ] The macOS application enrolls through the GUI, exchanges static node state, restarts, and reconnects without command-line arguments.
- [ ] P02 proof complete across Windows, Linux, and macOS.

### P03 — Hardware and network discovery

Prerequisites:

- [ ] Resolve A07 network benchmark and placement cost before P03 begins.

Build:

- [ ] Implement CPU discovery.
- [ ] Implement memory discovery.
- [ ] Implement disk discovery.
- [ ] Implement NVIDIA discovery through NVML on Windows.
- [ ] Implement NVIDIA discovery through NVML on Linux.
- [ ] Implement Apple Metal discovery on macOS.
- [ ] Implement capability reports.
- [ ] Implement the directional network benchmark defined by A07.
- [ ] Display hardware status in the GUI.
- [ ] Display peer status in the GUI.

Proof:

- [ ] A Windows NVIDIA peer truthfully reports measured hardware and network capabilities.
- [ ] A Linux NVIDIA peer truthfully reports measured hardware and network capabilities.
- [ ] A macOS Metal peer truthfully reports measured hardware and network capabilities.
- [ ] P03 proof complete across all required peer types.

### P04 — Automatic direct connectivity

Prerequisites:

- [ ] Resolve A08 router integration before automatic internet enrollment is complete.
- [ ] Resolve A10 peer-record merge rules before automatic internet enrollment is complete.

Build:

- [ ] Implement IPv6 candidate collection.
- [ ] Implement IPv4 candidate collection.
- [ ] Implement automatic router mapping.
- [ ] Implement peer-assisted hole punching.
- [ ] Implement guided firewall-failure recovery.
- [ ] Implement guided manual-forwarding failure recovery.

Proof:

- [ ] Enrollment uses automatic direct paths when available.
- [ ] Enrollment gives one clear recovery action when an automatic direct path is unavailable.
- [ ] P04 proof covers both available-path and unavailable-path cases.

### P05 — Resource reservations

Build:

- [ ] Implement resource offers.
- [ ] Implement expiring GPU leases.
- [ ] Implement expiring memory leases.
- [ ] Implement expiring disk leases.
- [ ] Implement expiring execution leases.
- [ ] Implement reservation commit.
- [ ] Implement reservation release.
- [ ] Handle concurrent coordinator conflicts.
- [ ] Display reservation state in the GUI.

Proof:

- [ ] Prove two coordinators cannot reserve the same local capacity.

### P06 — Model provider and cache

Prerequisites:

- [ ] Resolve A11 provider manifest generation.
- [ ] Resolve A12 partial download validation.
- [ ] Resolve A13 provider access and local cache.

Build:

- [ ] Define and implement the `ModelProvider` interface.
- [ ] Implement the Hugging Face adapter.
- [ ] Implement immutable revision resolution.
- [ ] Implement the local artifact cache.
- [ ] Implement the Safetensors metadata parser.
- [ ] Implement range downloads.
- [ ] Implement complete-shard fallback.
- [ ] Implement parallel node preparation.
- [ ] Add GUI model selection.
- [ ] Add GUI provider-access state and controls.
- [ ] Add GUI download progress.
- [ ] Add GUI failures for model selection, provider access, and downloads.

Proof:

- [ ] A selected Windows node automatically downloads its verified Qwen3-8B tensor assignment for one immutable revision.
- [ ] A selected Linux node automatically downloads its different verified Qwen3-8B tensor assignment for the same immutable revision.
- [ ] A selected macOS node automatically downloads its different verified Qwen3-8B tensor assignment for the same immutable revision.
- [ ] Selected Windows, Linux, and macOS nodes automatically download different verified Qwen3-8B tensor assignments for one immutable revision.

### P07 — Single-node inference

Prerequisites:

- [ ] Resolve A01 Qwen3 dense Model Family Adapter.
- [ ] Lock the A02 first correctness profile.
- [ ] Resolve A05 tokenizer and sampling ownership.
- [ ] Resolve A06 KV-cache contract.
- [ ] Resolve A07 network benchmark and placement cost.
- [ ] Resolve A11 provider manifest generation.
- [ ] Resolve A12 partial download validation.
- [ ] Resolve A13 provider access and local cache.

Build:

- [ ] Implement the dense Qwen3 Model Family Adapter.
- [ ] Implement a complete `Qwen/Qwen3-4B` stage.
- [ ] Implement and natively validate the Candle CUDA stage on Windows.
- [ ] Implement and natively validate the Candle CUDA stage on Linux.
- [ ] Implement and validate the Candle Metal stage on macOS Apple Silicon.
- [ ] Implement the Qwen3 tokenizer.
- [ ] Implement the non-thinking chat template.
- [ ] Implement the Qwen3 KV cache.
- [ ] Implement seeded sampling.
- [ ] Stream token output in the GUI.

Proof:

- [ ] The pinned Qwen3-4B model produces valid streamed output on Windows CUDA under the accepted correctness profile.
- [ ] The pinned Qwen3-4B model produces valid streamed output on Linux CUDA under the accepted correctness profile.
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

- [ ] Implement full-model replicas.
- [ ] Implement request routing.
- [ ] Implement dynamic batching.
- [ ] Implement per-node concurrency limits.
- [ ] Implement load reporting.
- [ ] Implement health reporting.

Proof:

- [ ] Independent Qwen3-4B requests run concurrently on separate complete-model nodes.

### P09 — Layer pipeline inference

Build:

- [ ] Implement layer placement.
- [ ] Implement partial stage loading.
- [ ] Implement the accepted activation wire format.
- [ ] Implement the pipeline stage runtime.
- [ ] Implement final-stage sampling.
- [ ] Implement concurrent sequences.
- [ ] Implement cancellation.
- [ ] Implement queue bounds.
- [ ] Implement backpressure.

Proof:

- [ ] The pinned Qwen3-8B model runs as continuous layer stages across at least two directly connected PCs, including a mixed Windows/Linux/macOS route.

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

- [ ] Begin P01.
- [ ] Begin the manually reachable path of P02.
- [ ] Select router-mapping crates before automatic internet enrollment is complete (A08).
- [ ] Resolve peer-record merge rules before automatic internet enrollment is complete (A10).
- [ ] Lock tokenizer and sampling before P07 inference (A05).
- [ ] Lock the KV-cache contract before P07 inference (A06).
- [ ] Lock network placement before P07 inference (A07).
- [ ] Lock provider manifest generation before P07 inference (A11).
- [ ] Lock provider download validation before P07 inference (A12).
- [ ] Lock provider access and cache behavior before P07 inference (A13).
