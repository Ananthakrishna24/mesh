# Goal
Design an extendable decentralized hardware mesh architecture for direct GPU compute across internet-connected PCs.

# Done
- Architecture docs and locked contracts through A11–A13; A05/A06 (`b3b4509`, `3e6e96a`).
- P01–P06 Linux paths (through `218e5de`).
- P07 single-node path + Linux CPU proof (`aa79a97`, `ebe19d8`).
- P07 Linux CUDA proof (`c49ad72`): Qwen3-4B prepare/load/generate `backend=cuda` on RTX 4070 SUPER.
- P08 full-model replica routing (`39b82e4`); handoff refresh commit `70b2ab0`.
- P09 foundation (`425bb5d`): activation frame, placement, mesh-owned `Qwen3Stage`, in-process pipeline, NextTokenFeedback, CUDA two-stage==complete match.
- P09 multi-node QUIC path + checklist/roadmap notes (`65febfb`).
- **Stage-filtered prepare/load (working tree, uncommitted):**
  - `build_stage_plan` / `stage_plan_flags` / `resolved_tie_word_embeddings` wrap `build_layer_plan` with Final+tied embed inclusion.
  - `materialize_stage_weight_files` turns prepared whole shards or pure range objects into mmap-able safetensors under `stages/…`.
  - `StageWorker::load_from_prepared` + in-process `PipelineEngine::load_in_process` use materialize (no longer hard-require whole-shard-only grouping for stages).
  - `LoadPipelineStage` prepares the local assignment via `build_stage_plan` + `prepare_plan` before load; complete `PrepareSelectedModel` path unchanged for P07/P08.
  - Unit proof: `download::tests::{layer_plan_filters_and_includes_tied_embed_for_final,prepare_layer_plan_uses_ranges_and_materializes}`.
  - Dual-node P09 smoke now probes only; stage prepare happens inside `LoadPipelineStage`.

# In progress
- Nothing mid-edit. Stage-filtered prepare/load implemented and unit-proven. Dual-node CUDA re-smoke optional (warm cache). Uncommitted on working tree.

# Decisions
- Phases are serial for roadmap proof gates; Linux implementation may continue while Windows/Metal host proofs wait.
- Almost no code comments; prefer explicit names/types.
- `roadmap.md` is canonical; `checklist.md` tracks progress only.
- eframe/egui **0.36**: `App::ui(&mut Ui)`, `egui::Panel::{top,bottom}`.
- Prost build uses `protoc-bin-vendored`.
- Candle **0.11** with in-tree `qwen3`; complete-stage still uses `ModelForCausalLM`. P09 partial stages are **mesh-owned** because Candle `DecoderLayer` is private — do not load full model and drop layers.
- Runtime dtype: CUDA/Metal FP16, CPU F32. Wire activations always little-endian FP16.
- Complete/replica path still uses whole-shard prepare (`build_complete_plan`) + `group_complete_weight_files`.
- Pipeline stages prepare only assigned tensors via `build_stage_plan`; load materializes ranges into stage safetensors when needed. Whole covering shards are reused as-is.
- Feature flags: `mesh-app --features cuda` → `mesh-node/cuda` → `mesh-inference/cuda` → `mesh-compute/cuda` (same for `metal`).
- P08: full-model replicas + control-plane remote generate. No activation frames.
- P09 cut order: (1) in-process multi-stage correctness ✓ (2) multi-node QUIC activation serving ✓ (3) stage-filtered weights ✓ (4) concurrent sequences / 8B.
- Dynamic batching deferred (`FIRST_MAX_CONCURRENT_REQUESTS = 1`).
- Replica route order: least `active_requests`, then prefer local, then `node_id`.
- Remote replica generate: coordinator tokenizes; worker samples; TokenResults currently after full generate (not live mid-token stream).
- Pipeline token feedback: final → first uses control `NextTokenFeedback` (not activation frame). Coordinator still owns tokenizer + user TokenResult stream.
- Multi-node pipeline uses developer-forced `PlacementPlan::split_even` via `UiCommand::LoadPipelineStage` (not automatic planner).
- **Two-PC user path today:**
  - **GUI-ready:** enroll + P08 full-model replica generate (`Load model` / `Generate` in `mesh-app`).
  - **Not GUI-ready:** P09 layer split. `LoadPipelineStage` is runtime/`UiCommand` only; no mesh-app button or placement UI yet. Proven via same-host dual-runtime smoke, not a polished two-PC product flow.
- Final stage is seeded with `InferenceRequest` (sampling params + prompt_len) before first activation; owner is the coordinator when known.
- Final-stage sampler prompt history currently uses a zero stub of `prompt_len` (enough for greedy/max_new_tokens; repetition penalty over real prompt tokens is weaker until real ids are retained).
- Durable smoke cache: `$HOME/mesh-p07-smoke` (~7.6G, Qwen3-4B whole shards + HF sidecars).
- User-local CUDA 13.1: `$HOME/cuda-root` + `source $HOME/cuda-env.sh`. glibc 2.43 needs `rsqrt`/`rsqrtf` `noexcept` patch on toolkit `crt/math_functions.h` (`.bak` kept).
- Final+tied embeddings: stage plan includes embed tensors for Final even when `owns_embeddings()` is false (Qwen3-4B is tied).

# Gotchas
- Host: NVIDIA driver + RTX 4070 SUPER; toolkit is user-local, not system `/usr/local/cuda`. Rebuilds need `source $HOME/cuda-env.sh`.
- Re-extracting CUDA debs into `$HOME/cuda-root` overwrites the rsqrt patch — re-apply from `.bak` / patch again.
- P08 dual-node smokes symlink `model-cache` + `cache` from `$HOME/mesh-p07-smoke` into temp dirs. P09 multi smoke does the same; stage prepare can reuse whole shards already in the shared cache.
- One 12GB GPU can hold two half Qwen3-4B FP16 stages for dual-node same-host smoke; two full replicas still need a second host / two GPUs.
- Dual-node P09 smoke wall time was ~11 min with complete prepare; stage-filtered path skips the extra complete prepare step but load still dominates on cold GPU.
- Activation streams share the peer QUIC connection with bandwidth benchmarks; after handshake, `accept_uni` is treated as activation frames.
- `SessionEvent::NextTokenFeedback` is handled for first-stage decode; unknown/stale feedback is ignored.
- Flow: Probe → (optional complete Prepare for P07/P08) → Load / LoadPipelineStage (stage prepare+load) → Generate.
- Complete-stage `LoadSelectedModel` still fails if only range artifacts exist — use complete prepare for replicas.
- Two-PC P08 needs: direct QUIC path, worker loads full model, coordinator has tokenizer (prepare/load once), then Generate on coordinator.
- Two-PC P09 needs shared `deployment_id` + matching `PlacementPlan` on both nodes and a way to issue `LoadPipelineStage` (test/driver; not GUI). Stage prepare no longer requires a prior complete Prepare on each node (probe+resolve is enough).
- `UiCommand` is not `Eq` (`Generate` carries `f32`).
- Snapshot resets must preserve `models`, `resources`, `inference`.
- Qwen3 non-thinking template requires empty think block suffix.
- HF LFS digest is origin `x-linked-etag`, not CDN ETag after redirect.
- Qwen3-4B shards: 00001=layers0–15+embed, 00002=layers15–35, 00003=layer35+norm; Final+tied still touches embed shard. Range vs complete choice remains 80% coverage rule inside `prepare_plan`.

# Next
1. **Optional:** re-run dual-node P09 CUDA smoke to confirm stage-prepare path end-to-end (`MESH_P09_MULTI_SMOKE=1`…).
2. **Optional UX:** mesh-app controls for `LoadPipelineStage` / forced 2-stage placement so two PCs can run P09 without a custom driver.
3. Concurrent sequences in the pipeline (`max_concurrent_requests` > 1, ordered queues by request/seq).
4. Qwen3-8B distributed proof (real capacity split benefits most from stage-filtered prepare).
5. Optional polish: retain real prompt token ids on final-stage sampler history.
6. Host proofs when machines available: Windows CUDA / **P07.5**, macOS Metal; mixed-route P09. Do not block Linux 8B on them.
7. Commit stage-filtered prepare/load (and prior multi-node P09 docs if needed) when ready.

# Quick commands
```bash
# CUDA env (required for --features cuda)
source $HOME/cuda-env.sh

# Stage-filtered unit proof
cargo test -p mesh-model --lib download::tests -- --nocapture

# GUI CUDA (reuses cache) — two-PC: enroll, worker Load model, coord Generate (P08)
MESH_DATA_DIR=$HOME/mesh-p07-smoke cargo run -p mesh-app --release --features cuda

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

# P09 dual-node QUIC pipeline smoke (CUDA) — stage prepare inside LoadPipelineStage
MESH_P09_MULTI_SMOKE=1 MESH_P07_DATA_DIR=$HOME/mesh-p07-smoke MESH_P09_MAX_NEW_TOKENS=4 \
  cargo test -p mesh-node --lib --release --features cuda \
  runtime::tests::p09_dual_node_pipeline_generate_smoke -- --exact --nocapture
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
| `docs/architecture/inference/model-distribution.md` | Partial ranges + complete-shard fallback |
| `apps/mesh-app/src/app.rs` | GUI: invite, Load model, Generate (no pipeline stage UI yet) |
| `crates/mesh-core/src/activation.rs` | Activation header |
| `crates/mesh-core/src/inference.rs` | Placement + NextTokenFeedback |
| `crates/mesh-core/src/ui.rs` | `UiCommand::LoadPipelineStage` |
| `crates/mesh-core/proto/mesh/v1/control.proto` | NextTokenFeedback = 44 |
| `crates/mesh-compute/src/qwen3_stage.rs` | Mesh-owned partial stage |
| `crates/mesh-inference/src/pipeline.rs` | PipelineEngine + StageWorker hop API + materialize load |
| `crates/mesh-inference/src/engine.rs` | SingleNodeEngine + `locate_sidecar` (complete path) |
| `crates/mesh-net/src/activation.rs` | Activation stream I/O |
| `crates/mesh-net/src/session.rs` | Control session + activation accept/send |
| `crates/mesh-net/src/inference.rs` | Control envelope builders |
| `crates/mesh-node/src/runtime.rs` | Multi-node pipeline runtime + dual-node smoke |
| `crates/mesh-model/src/download.rs` | `build_complete_plan` / `build_layer_plan` / `build_stage_plan` / `materialize_stage_weight_files` |
| `$HOME/mesh-p07-smoke` | Durable prepare/load cache |
| `$HOME/cuda-root` + `$HOME/cuda-env.sh` | User-local CUDA 13.1 |
| `$HOME/cuda-debs` | Cached debs for toolkit re-extract |
