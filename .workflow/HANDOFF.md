# Goal
Design an extendable decentralized hardware mesh architecture for direct GPU compute across internet-connected PCs.

# Done
- Architecture docs and locked contracts through A11–A13; A05/A06 (`b3b4509`, `3e6e96a`).
- P01–P06 Linux paths (through `218e5de`).
- P07 Linux implementation foundation (`aa79a97`).
- **P07 Linux CPU host proof** (`ebe19d8`):
  - HF metadata: no-redirect meta client; prefer origin `x-linked-etag` / `x-linked-size` over CDN ETag (fixes complete-shard digest mismatch).
  - Official Qwen3 non-thinking template suffix: `<|im_start|>assistant\n<think>\n\n</think>\n\n`.
  - Gated smoke: `runtime::tests::p07_single_node_prepare_load_generate_smoke` (`MESH_P07_SMOKE=1`).
  - Host evidence: prepare 7.5 GiB Qwen3-4B@`1cfa9a720891`, load `backend=cpu`, generate `tokens=3 stop=eos output="Hello!"`.
  - Checklist/roadmap updated with Linux CPU evidence notes (CUDA/Metal still open).

# In progress
- Nothing mid-edit. Working tree clean after `ebe19d8`.

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
- Durable P07 smoke data lives under `$HOME/mesh-p07-smoke` (not `/tmp`; tmpfs is too small for ~8GB weights).
- HF LFS content digest is `x-linked-etag` on the origin response; CDN ETag after redirect is not SHA-256 of the file.
- Qwen3 non-thinking requires the empty think block in the generation prompt; bare `assistant\n` still thinks.

# Gotchas
- Host has NVIDIA driver + `libcuda` but **no CUDA toolkit/`nvcc`**. Default build is CPU. `--features cuda` needs toolkit installed.
- `UiCommand` is no longer `Eq` (`Generate` carries `f32`).
- First-run / cancel-enrollment snapshot resets must preserve `models`, `resources`, and **`inference`**.
- Engine `stop_reason` must be assigned on every generate loop exit path.
- Qwen3-4B ~8GB FP16 weights; CPU F32 load uses more RAM; generate is slow on CPU without cache-hit warm path.
- Flow: Probe/resolve → Prepare downloads → Load model → Generate. Load fails without prepare.
- Load looks up `tokenizer.json`/`config.json` under mesh HF cache `cache/hf-hub/models--Qwen--Qwen3-4B/snapshots/<sha>/` (and `HF_HOME` / `~/.cache/huggingface/hub`).
- If only range artifacts exist in model-cache, load errors asking for whole shards — re-prepare complete plan.
- Live HF download needs network; multi-GB. Reuse `MESH_P07_DATA_DIR=$HOME/mesh-p07-smoke` for cache hits.
- Gated smoke reuses restored mesh identity when `mesh.db` already exists (do not require `AwaitingOnboarding`).
- Branch is **ahead of origin** by at least `ebe19d8`; push when ready.

# Next
1. When CUDA toolkit is available:
   ```bash
   MESH_DATA_DIR=$HOME/mesh-p07-smoke cargo run -p mesh-app --release --features cuda
   ```
   or re-run gated smoke with a CUDA-enabled build and record Linux CUDA checklist evidence.
2. Optional dual-window enrollment anytime (`/tmp/mesh-a`, `/tmp/mesh-b`).
3. Do **not** start P08 until a required backend proof is accepted; Linux CPU is host evidence only, not the CUDA checklist item.
4. After Linux CUDA (or accepted path): macOS Metal / Windows CUDA; then **P07.5**.
5. Push commits when ready.

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
| `crates/mesh-model/src/huggingface.rs` | HF provider + LFS metadata |
| `crates/mesh-core/src/inference.rs` | Shared inference types |
| `crates/mesh-node/src/runtime.rs` | Load/generate wiring + P07 smoke |
| `apps/mesh-app/src/app.rs` | Inference card |
| `$HOME/mesh-p07-smoke` | Durable prepare cache for host smoke |
