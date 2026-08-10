# Goal
Design an extendable decentralized hardware mesh architecture for direct GPU compute across internet-connected PCs.

# Done
- Architecture docs and locked contracts through A11–A13; A05/A06 (`b3b4509`, `3e6e96a`).
- P01–P06 Linux paths (through `218e5de`).
- P07 Linux implementation foundation (`aa79a97`).
- P07 Linux CPU host proof (`ebe19d8`, handoff `c3bf3fb` / `d978cf0`):
  - prepare/load/generate on CPU; durable cache `$HOME/mesh-p07-smoke`.
- **P07 Linux CUDA host proof** (this session):
  - User-local CUDA 13.1 toolkit at `$HOME/cuda-root` (dpkg-deb extract; no root). Env: `source $HOME/cuda-env.sh`.
  - glibc 2.43 vs CUDA 13.1: patched `crt/math_functions.h` `rsqrt`/`rsqrtf` with `noexcept` (backup `.bak`).
  - Gated smoke: `MESH_P07_SMOKE=1 MESH_P07_DATA_DIR=$HOME/mesh-p07-smoke MESH_P07_MAX_NEW_TOKENS=16 cargo test -p mesh-node --lib --release --features cuda runtime::tests::p07_single_node_prepare_load_generate_smoke -- --exact --nocapture`
  - Result: prepare cache-hit 7.5 GiB, load `backend=cuda`, generate `tokens=3 stop=eos output="Hello!"` (~15s wall, RTX 4070 SUPER).
  - Checklist: Linux CUDA P07 proof checked; baseline "Native NVIDIA CUDA support on Linux" checked.

# In progress
- Nothing mid-edit. Next session starts at Next #1.

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
- Linux CUDA proof accepted via gated smoke (`backend=cuda`); Metal/Windows still open. Do not start P08 until roadmap-required backends for the next gate are accepted — Linux CUDA unblocks continuing host proofs, not automatic P08.
- Host CUDA toolkit is **user-local** (`$HOME/cuda-root` + `$HOME/cuda-env.sh`), not system `/usr/local/cuda`. Rebuilds need that env.
- CUDA 13.1 + Ubuntu glibc 2.43 needs the local `rsqrt`/`rsqrtf` noexcept header patch under the user toolkit tree.

# Gotchas
- Host has NVIDIA driver 595 / CUDA driver API 13.2 + RTX 4070 SUPER. Toolkit is user-local 13.1 (`nvcc` via `source $HOME/cuda-env.sh`). System apt install needs sudo password (not used).
- `UiCommand` is no longer `Eq` (`Generate` carries `f32`).
- First-run / cancel-enrollment snapshot resets must preserve `models`, `resources`, and **`inference`**.
- Engine `stop_reason` must be assigned on every generate loop exit path.
- Qwen3-4B ~8GB FP16 weights; CUDA FP16 fits 12GB card with headroom; CPU F32 uses more RAM.
- Flow: Probe/resolve → Prepare downloads → Load model → Generate. Load fails without prepare.
- Load looks up `tokenizer.json`/`config.json` under mesh HF cache `cache/hf-hub/models--Qwen--Qwen3-4B/snapshots/<sha>/` (and `HF_HOME` / `~/.cache/huggingface/hub`).
- If only range artifacts exist in model-cache, load errors asking for whole shards — re-prepare complete plan.
- Live HF download needs network; multi-GB. Reuse `MESH_P07_DATA_DIR=$HOME/mesh-p07-smoke` for cache hits.
- Gated smoke reuses restored mesh identity when `mesh.db` already exists (do not require `AwaitingOnboarding`).
- Without `source $HOME/cuda-env.sh`, `--features cuda` builds fail (missing nvcc / libs).
- Re-extracting CUDA debs from `$HOME/cuda-debs` overwrites the patched `math_functions.h` — re-apply noexcept patch or restore from `.bak` then re-patch.
- `$HOME/cuda-debs` (~1GB) and `$HOME/cuda-root` (~2.3GB) are outside the repo; keep them for rebuilds.
- Partial CUDA 12.8 runfile download was cancelled/removed after 13.1 path worked.

# Next
1. Optional: dual-window enrollment anytime (`/tmp/mesh-a`, `/tmp/mesh-b`).
2. macOS Metal host proof when a Mac is available; Windows CUDA / **P07.5** on a Windows NVIDIA host.
3. Do **not** start P08 until the roadmap’s required backend proofs for leaving P07 are accepted (Linux CUDA is done; Metal/Windows still open for full P07 checkbox).
4. Optional GUI CUDA path: `source $HOME/cuda-env.sh && MESH_DATA_DIR=$HOME/mesh-p07-smoke cargo run -p mesh-app --release --features cuda` → Probe → Prepare (cache hit) → Load → Generate.
5. Push docs commit when ready.
6. If toolkit is lost: re-extract debs from `$HOME/cuda-debs` into `$HOME/cuda-root`, re-apply rsqrt noexcept patch, keep `$HOME/cuda-env.sh`.

# Resume map
| Path | Role |
|---|---|
| `AGENTS.md` | Agent coding/phase rules |
| `docs/implementation/roadmap.md` | Canonical phase order |
| `docs/implementation/checklist.md` | Progress checkboxes + P07 CPU/CUDA evidence |
| `docs/architecture/inference/tokenizer-and-sampling.md` | A05 |
| `docs/architecture/inference/kv-cache.md` | A06 |
| `docs/decisions/0016-tokenizer-sampling-kv-cache.md` | A05/A06 ADR |
| `crates/mesh-compute/src/lib.rs` | Candle Qwen3 complete stage + device select |
| `crates/mesh-inference/src/engine.rs` | Single-node load/generate |
| `crates/mesh-inference/src/sampler.rs` | A05 sampler |
| `crates/mesh-inference/src/tokenizer.rs` | Non-thinking template + HF tokenizers |
| `crates/mesh-model/src/huggingface.rs` | HF provider + LFS metadata fix |
| `crates/mesh-core/src/inference.rs` | Shared inference types |
| `crates/mesh-node/src/runtime.rs` | Load/generate wiring + gated P07 smoke |
| `apps/mesh-app/src/app.rs` | Inference card |
| `$HOME/mesh-p07-smoke` | Durable prepare/load cache for host smoke |
| `$HOME/cuda-root` + `$HOME/cuda-env.sh` | User-local CUDA 13.1 toolkit for `--features cuda` |
| `$HOME/cuda-debs` | Cached CUDA 13.1 debs for re-extract |
