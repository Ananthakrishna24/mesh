# Goal
Design an extendable decentralized hardware mesh architecture for direct GPU compute across internet-connected PCs.

# Done
- Architecture docs and locked contracts through A11–A13; A05/A06 (`b3b4509`, `3e6e96a`).
- P01–P06 Linux paths (through `218e5de`).
- **P07 Linux implementation foundation** (`aa79a97`):
  - `mesh-core`: `RequestId`, inference types (`SamplingParams`, `TokenResultEvent`, `InferenceView`), KV estimate helpers, UI Load/Generate/Cancel, proto fields 30–33 and 40–42.
  - `mesh-compute`: Candle 0.11 Qwen3 complete-stage load (`LoadedQwen3`), CPU default, optional `cuda`/`metal` features, whole-shard safetensors mmap.
  - `mesh-inference`: A05 sampler (ChaCha12, penalty→T→top-k→top-p), non-thinking chat template + `tokenizers`, `SingleNodeEngine` load/warmup/generate.
  - `mesh-node` / `mesh-app` / `mesh-inference`: `cuda`/`metal` feature passthrough (`--features cuda` on `mesh-app`).
  - `mesh-node`: prepare result retained; load/generate runtime events; engine returned via `GenerationFinished`.
  - `mesh-app`: Inference card (load/unload/generate/cancel, prompt, output).
  - Checklist/roadmap updated for partial P07 (build items checked; host proofs open).
  - Verified: `cargo test -p mesh-core -p mesh-inference -p mesh-compute -p mesh-node --lib`; `cargo build -p mesh-app`.

# In progress
- Nothing mid-edit. Working tree clean. Next is P07 host proof (prepare + load + generate).

# Decisions
- Phases are serial. Do not parallelize implementation phases (`AGENTS.md`).
- Almost no code comments; prefer explicit names/types.
- `roadmap.md` is canonical; `checklist.md` tracks progress only.
- eframe/egui **0.36**: implement `App::ui(&mut Ui)`, use `egui::Panel::{top,bottom}`.
- Prost build uses `protoc-bin-vendored`.
- Candle **0.11** with in-tree `qwen3`; complete-stage uses full `ModelForCausalLM`. True partial layer construct remains P09.
- Runtime dtype: CUDA/Metal FP16, CPU F32.
- Complete-stage load requires **whole-shard** cache objects (range-only prepare is not enough for mmap `VarBuilder`).
- Tokenizer/config sidecars resolved from HF hub cache snapshots by repo+revision (not only model-cache).
- Generation runs on `spawn_blocking`; engine moves out and returns on `GenerationFinished { prompt, engine, result }`.
- Default GUI generate uses temperature 0 for deterministic smoke.
- Feature flags: `mesh-app --features cuda` → `mesh-node/cuda` → `mesh-inference/cuda` → `mesh-compute/cuda` (same for `metal`).
- Windows still starts at **P07.5**. No parallel Windows track.
- Prior A05–A13 / P04–P06 decisions unchanged (git history).

# Gotchas
- Host has NVIDIA driver + `libcuda` but **no CUDA toolkit/`nvcc`**. Default build is CPU. `--features cuda` needs toolkit installed.
- `UiCommand` is no longer `Eq` (`Generate` carries `f32`).
- First-run / cancel-enrollment snapshot resets must preserve `models`, `resources`, and **`inference`**.
- Engine `stop_reason` must be assigned on every generate loop exit path.
- Qwen3-4B ~8GB FP16 weights; CPU generate is slow.
- Flow: Probe/resolve → Prepare downloads → Load model → Generate. Load fails without prepare.
- Load looks up `tokenizer.json`/`config.json` under HF hub cache `models--Qwen--Qwen3-4B/snapshots/<sha>/` (and `HF_HOME` / `~/.cache/huggingface/hub`).
- If only range artifacts exist in model-cache, load errors asking for whole shards — re-prepare complete plan.
- Live HF download needs network; multi-GB.
- Branch is **3 commits ahead of origin** (`b3b4509`, `3e6e96a`, `aa79a97`); push when ready.

# Next
1. Host smoke (network + disk), CPU path:
   ```bash
   MESH_DATA_DIR=/tmp/mesh-a cargo run --release
   ```
   Select Qwen3-4B → Check access → Probe/resolve → Prepare downloads → Load model → Generate.
2. When CUDA toolkit is available:
   ```bash
   MESH_DATA_DIR=/tmp/mesh-a cargo run -p mesh-app --release --features cuda
   ```
3. Record Linux generate evidence under checklist P07 proof (CPU note and/or CUDA).
4. Optional dual-window enrollment anytime (`/tmp/mesh-a`, `/tmp/mesh-b`).
5. Do **not** start P08 until a Linux generate proof works.
6. After Linux P07 path is proven: macOS Metal / Windows CUDA; then **P07.5**.

# Resume map
| Path | Role |
|---|---|
| `AGENTS.md` | Agent coding/phase rules |
| `docs/implementation/roadmap.md` | Canonical phase order |
| `docs/implementation/checklist.md` | Progress checkboxes |
| `docs/architecture/inference/tokenizer-and-sampling.md` | A05 |
| `docs/architecture/inference/kv-cache.md` | A06 |
| `docs/decisions/0016-tokenizer-sampling-kv-cache.md` | A05/A06 ADR |
| `crates/mesh-compute/src/lib.rs` | Candle Qwen3 complete stage |
| `crates/mesh-inference/src/engine.rs` | Single-node load/generate |
| `crates/mesh-inference/src/sampler.rs` | A05 sampler |
| `crates/mesh-inference/src/tokenizer.rs` | Template + HF tokenizers |
| `crates/mesh-core/src/inference.rs` | Shared inference types |
| `crates/mesh-node/src/runtime.rs` | Load/generate wiring |
| `apps/mesh-app/src/app.rs` | Inference card |
