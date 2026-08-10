# Goal
Design an extendable decentralized hardware mesh architecture for direct GPU compute across internet-connected PCs.

# Done
- Architecture docs and locked contracts through A11–A13; A05/A06 (`b3b4509`, `3e6e96a`).
- P01–P06 Linux paths (prior commits through `218e5de`).
- **P07 Linux implementation foundation** (commit this session):
  - `mesh-core`: `RequestId`, inference types (`SamplingParams`, `TokenResultEvent`, `InferenceView`), KV estimate helpers, UI commands Load/Generate/Cancel, proto fields 30–33 and 40–42.
  - `mesh-compute`: Candle 0.11 Qwen3 complete-stage load (`LoadedQwen3`), CPU default, optional `cuda`/`metal` features, whole-shard safetensors mmap.
  - `mesh-inference`: A05 sampler (ChaCha12, penalty→T→top-k→top-p), non-thinking chat template + `tokenizers`, `SingleNodeEngine` load/warmup/generate.
  - `mesh-node`: prepare result retained; load/generate runtime events; engine ownership across generation.
  - `mesh-app`: Inference card (load/unload/generate/cancel, prompt, output).
  - Verified: `cargo test -p mesh-core -p mesh-inference -p mesh-compute -p mesh-node --lib`; `cargo build -p mesh-app`.

# In progress
- P07 host proof still open: real Qwen3-4B prepare + generate on this machine. CUDA toolkit not installed (driver only), so load prefers CUDA feature when built with `--features cuda` else CPU.

# Decisions
- Phases are serial. Do not parallelize implementation phases (`AGENTS.md`).
- Almost no code comments; prefer explicit names/types.
- `roadmap.md` is canonical; `checklist.md` tracks progress only.
- eframe/egui **0.36**: implement `App::ui(&mut Ui)`, use `egui::Panel::{top,bottom}`.
- Prost build uses `protoc-bin-vendored`.
- Candle **0.11** with in-tree `qwen3` module; complete-stage uses `ModelForCausalLM` (full construct). True partial layer construct remains P09.
- Runtime dtype: CUDA/Metal FP16, CPU F32 (Candle Qwen3 rejects f64; CPU flash path is f32).
- Complete-stage load requires **whole-shard** cache objects (range-only prepare is not enough for mmap VarBuilder).
- Tokenizer/config sidecars resolved from HF hub cache snapshots by repo+revision (not only model-cache).
- Generation runs on `spawn_blocking`; engine is moved out and returned via `GenerationFinished`.
- Default GUI generate uses temperature 0 for deterministic smoke.
- Windows still starts at P07.5. No parallel Windows track.
- Prior A05–A13 / P04–P06 decisions unchanged (see previous handoff bodies in git).

# Gotchas
- Host has NVIDIA driver + `libcuda` but **no CUDA toolkit/`nvcc`**; building `--features cuda` needs toolkit. Without it, prefer default CPU feature set.
- `UiCommand` is no longer `Eq` because `Generate` carries `f32`.
- First-run / cancel-enrollment snapshot resets must preserve `models`, `resources`, and **`inference`**.
- `stop_reason` in engine must be assigned on every loop exit path.
- Qwen3-4B ~8GB FP16; CPU generate will be slow; prefer CUDA when toolkit present.
- Prepare must finish before Load; Load looks up `tokenizer.json`/`config.json` under HF hub cache `models--Qwen--Qwen3-4B/snapshots/<sha>/`.
- If only range artifacts exist in model-cache, load errors asking for whole shards — re-prepare complete plan (coverage usually pulls complete shards for dense models).
- Live HF download needs network; multi-GB.

# Next
1. Host smoke (network + disk):
   ```bash
   MESH_DATA_DIR=/tmp/mesh-a cargo run --release
   ```
   Select Qwen3-4B → Check access → Probe/resolve → Prepare downloads → Load model → Generate.
2. When CUDA toolkit available:
   ```bash
   cargo run -p mesh-app --release --features mesh-compute/cuda
   ```
   (wire feature through mesh-app/mesh-node if not already default-propagated — may need Cargo feature passthrough).
3. Add feature passthrough on `mesh-node`/`mesh-app` for `cuda` if missing.
4. Record Linux CUDA generate evidence under checklist P07 proof.
5. Do not start P08 until Linux generate proof works.

# Resume map
| Path | Role |
|---|---|
| `crates/mesh-compute/src/lib.rs` | Candle Qwen3 complete stage |
| `crates/mesh-inference/src/engine.rs` | Single-node load/generate |
| `crates/mesh-inference/src/sampler.rs` | A05 sampler |
| `crates/mesh-inference/src/tokenizer.rs` | Template + HF tokenizers |
| `crates/mesh-core/src/inference.rs` | Shared inference types |
| `crates/mesh-node/src/runtime.rs` | Load/generate wiring |
| `apps/mesh-app/src/app.rs` | Inference card |
| `docs/architecture/inference/tokenizer-and-sampling.md` | A05 |
| `docs/architecture/inference/kv-cache.md` | A06 |
