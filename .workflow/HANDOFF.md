# Goal
Design an extendable decentralized hardware mesh architecture for direct GPU compute across internet-connected PCs.

# Done
- Architecture docs and locked contracts through A11–A13; A05/A06 (`b3b4509`, `3e6e96a`).
- P01–P06 Linux paths (through `218e5de`).
- P07 single-node path + Linux CPU proof (`aa79a97`, `ebe19d8`).
- P07 Linux CUDA proof (`c49ad72`): Qwen3-4B prepare/load/generate `backend=cuda` on RTX 4070 SUPER.
- P08 full-model replica routing (`39b82e4`); handoff refresh commit `70b2ab0`.
- **P09 foundation (uncommitted on working tree atop `70b2ab0`):**
  - Activation wire: `crates/mesh-core/src/activation.rs` + `crates/mesh-net/src/activation.rs`.
  - Placement: `StageRole`, `LayerRange`, `StageAssignment`, `PlacementPlan` in `mesh-core/src/inference.rs`.
  - Partial stage: mesh-owned `Qwen3Stage` in `crates/mesh-compute/src/qwen3_stage.rs`.
  - Pipeline runtime: `PipelineEngine` / `StageWorker` in `crates/mesh-inference/src/pipeline.rs` (in-process multi-stage, final-stage sampling, cancel, queue bound=2, backpressure).
  - Control: `NextTokenFeedback` proto field 44 + net builders + session command/event; node match arm stubs ignore for now.
  - Checklist/roadmap current-state + P09 partial checks updated.
  - Units: placement split/gap, activation header round-trip + duplicate reject, bounded queue full.
  - Gated CUDA proof: `pipeline::tests::p09_two_stage_matches_complete_greedy` → two-stage == complete greedy tokens, `backend=cuda`, `text="Hello! How can"`.

# In progress
- Nothing mid-edit. Working tree has **uncommitted P09 foundation**. Next session starts at Next #1.

# Decisions
- Phases are serial for roadmap proof gates; Linux implementation may continue while Windows/Metal host proofs wait.
- Almost no code comments; prefer explicit names/types.
- `roadmap.md` is canonical; `checklist.md` tracks progress only.
- eframe/egui **0.36**: `App::ui(&mut Ui)`, `egui::Panel::{top,bottom}`.
- Prost build uses `protoc-bin-vendored`.
- Candle **0.11** with in-tree `qwen3`; complete-stage still uses `ModelForCausalLM`. P09 partial stages are **mesh-owned** because Candle `DecoderLayer` is private — do not load full model and drop layers.
- Runtime dtype: CUDA/Metal FP16, CPU F32. Wire activations always little-endian FP16.
- Complete-stage and current P09 stage load still need **whole-shard** cache objects. Stage-filtered prepare/load is still open.
- Feature flags: `mesh-app --features cuda` → `mesh-node/cuda` → `mesh-inference/cuda` → `mesh-compute/cuda` (same for `metal`).
- P08: full-model replicas + control-plane remote generate. No activation frames.
- P09 cut order: (1) in-process multi-stage correctness ✓ (2) multi-node QUIC activation serving (3) stage-filtered weights / 8B (4) concurrent sequences.
- Dynamic batching deferred (`FIRST_MAX_CONCURRENT_REQUESTS = 1`).
- Replica route order: least `active_requests`, then prefer local, then `node_id`.
- Remote replica generate: coordinator tokenizes; worker samples; TokenResults currently after full generate (not live mid-token stream).
- Pipeline token feedback: final → first uses control `NextTokenFeedback` (not activation frame). Coordinator still owns tokenizer + user TokenResult stream.
- Durable smoke cache: `$HOME/mesh-p07-smoke` (~7.6G, Qwen3-4B whole shards + HF sidecars).
- User-local CUDA 13.1: `$HOME/cuda-root` + `source $HOME/cuda-env.sh`. glibc 2.43 needs `rsqrt`/`rsqrtf` `noexcept` patch on toolkit `crt/math_functions.h` (`.bak` kept).

# Gotchas
- Host: NVIDIA driver + RTX 4070 SUPER; toolkit is user-local, not system `/usr/local/cuda`. Rebuilds need `source $HOME/cuda-env.sh`.
- Re-extracting CUDA debs into `$HOME/cuda-root` overwrites the rsqrt patch — re-apply from `.bak` / patch again.
- P08 dual-node smoke symlinks `model-cache` + `cache` from `$HOME/mesh-p07-smoke` into temp dirs.
- One 12GB GPU cannot hold two full Qwen3-4B FP16 replicas; concurrent two-replica proof needs a second host.
- P09 two-stage in-process proof loads **both** stages on one GPU from the same whole shards; peak VRAM > one complete model briefly. Still correct for token-match proof, not a memory-split proof.
- Activation I/O helpers exist in `mesh-net` but are **not yet driven** from `mesh-node` session loops / incoming uni-streams.
- `SessionEvent::NextTokenFeedback` is matched in `mesh-node` as a no-op stub — wire real first-stage decode feedback when multi-node pipeline lands.
- Flow still: Probe → Prepare → (optional Load) → Generate. Remote route needs coordinator tokenizer via prepare (or prior load).
- Load fails if only range artifacts exist — re-prepare complete plan for whole shards.
- `UiCommand` is not `Eq` (`Generate` carries `f32`).
- Snapshot resets must preserve `models`, `resources`, `inference`.
- Qwen3 non-thinking template requires empty think block suffix.
- HF LFS digest is origin `x-linked-etag`, not CDN ETag after redirect.

# Next
1. **Default implementation next: multi-node P09 path**
   - Accept incoming QUIC unidirectional activation streams in `mesh-node` / session layer.
   - Send activations on the direct peer connection to the next stage (`send_activation_on_connection`).
   - Load one `StageWorker`/`Qwen3Stage` per node from `PlacementPlan` assignment.
   - Control path: coordinator → first `InferenceRequest`; final → coordinator `TokenResult`; final → first `NextTokenFeedback` for decode steps; `CancelRequest` to all stages.
   - Prefer a gated dual-node smoke on Qwen3-4B (developer forced 2-stage split) before 8B.
2. Stage-filtered prepare/load (`build_layer_plan` / assigned ranges only) so each node holds only its tensors — required for real capacity split and Qwen3-8B.
3. Concurrent sequences in the pipeline (`max_concurrent_requests` > 1, ordered queues by request/seq).
4. Optional P08 polish: live per-token `TokenResult`; dynamic batching after multi-slot engine.
5. Host proofs when machines available: Windows CUDA / **P07.5**, macOS Metal; mixed-route P09. Do not block Linux multi-node P09 on them.
6. Commit P09 foundation (and push with prior commits if not on origin) when ready.

# Quick commands
```bash
# CUDA env (required for --features cuda)
source $HOME/cuda-env.sh

# P07 single-node CUDA smoke
MESH_P07_SMOKE=1 MESH_P07_DATA_DIR=$HOME/mesh-p07-smoke MESH_P07_MAX_NEW_TOKENS=16 \
  cargo test -p mesh-node --lib --release --features cuda \
  runtime::tests::p07_single_node_prepare_load_generate_smoke -- --exact --nocapture

# P08 dual-node remote replica smoke
MESH_P08_SMOKE=1 MESH_P07_DATA_DIR=$HOME/mesh-p07-smoke MESH_P08_MAX_NEW_TOKENS=8 \
  cargo test -p mesh-node --lib --release --features cuda \
  runtime::tests::p08_remote_replica_generate_smoke -- --exact --nocapture

# P09 two-stage == complete greedy match (CUDA) — in-process foundation proof
MESH_P09_SMOKE=1 MESH_P07_DATA_DIR=$HOME/mesh-p07-smoke \
  cargo test -p mesh-inference --lib --release --features cuda \
  pipeline::tests::p09_two_stage_matches_complete_greedy -- --exact --nocapture

# GUI CUDA (reuses cache)
MESH_DATA_DIR=$HOME/mesh-p07-smoke cargo run -p mesh-app --release --features cuda
```

# Resume map
| Path | Role |
|---|---|
| `AGENTS.md` | Agent coding/phase rules |
| `docs/implementation/roadmap.md` | Canonical phase order |
| `docs/implementation/checklist.md` | Progress + P07/P08/P09 evidence notes |
| `docs/architecture/protocol/activation-frame.md` | P09 wire format |
| `docs/architecture/inference/parallelism-and-edge-cases.md` | Replica vs pipeline rules |
| `docs/architecture/inference/qwen3-model-family.md` | Stage roles / partial construct |
| `docs/architecture/inference/tokenizer-and-sampling.md` | A05 + NextTokenFeedback ownership |
| `docs/architecture/inference/kv-cache.md` | A06 |
| `crates/mesh-core/src/activation.rs` | Activation header |
| `crates/mesh-core/src/inference.rs` | Placement + NextTokenFeedback |
| `crates/mesh-core/proto/mesh/v1/control.proto` | NextTokenFeedback = 44 |
| `crates/mesh-compute/src/qwen3_stage.rs` | Mesh-owned partial stage |
| `crates/mesh-compute/src/lib.rs` | LoadedQwen3 complete path + exports |
| `crates/mesh-inference/src/pipeline.rs` | PipelineEngine + P09 smoke |
| `crates/mesh-inference/src/engine.rs` | SingleNodeEngine + `locate_sidecar` |
| `crates/mesh-net/src/activation.rs` | Activation stream I/O (not node-wired yet) |
| `crates/mesh-net/src/session.rs` | Control session; NextTokenFeedback wired |
| `crates/mesh-net/src/inference.rs` | Control envelope builders |
| `crates/mesh-node/src/runtime.rs` | Node runtime — multi-node P09 still open |
| `crates/mesh-model/src/download.rs` | `build_complete_plan` / `build_layer_plan` for stage-filtered prepare |
| `$HOME/mesh-p07-smoke` | Durable prepare/load cache |
| `$HOME/cuda-root` + `$HOME/cuda-env.sh` | User-local CUDA 13.1 |
| `$HOME/cuda-debs` | Cached debs for toolkit re-extract |
