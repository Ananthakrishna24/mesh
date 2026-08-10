# Goal
Design an extendable decentralized hardware mesh architecture for direct GPU compute across internet-connected PCs.

# Done
- Architecture docs and locked contracts through A11–A13; A05/A06 (`b3b4509`, `3e6e96a`).
- P01–P06 Linux paths (through `218e5de`).
- P07 single-node path + Linux CPU proof (`aa79a97`, `ebe19d8`).
- P07 Linux CUDA proof (`c49ad72`): Qwen3-4B prepare/load/generate `backend=cuda` on RTX 4070 SUPER.
- **P08 full-model replica routing** (`39b82e4`):
  - `ReplicaStatus` proto + `mesh-net` wire builders/session events.
  - Peer replica registry; least-loaded route (`select_replica_route`).
  - Per-node concurrency slots, load/health advertise.
  - Remote `InferenceRequest` / `TokenResult` / cancel path.
  - Coordinator tokenizer (prepare/load sidecars) so coord can route without local weights.
  - GUI replica list + routed node + peer replica line.
  - Unit: `select_replica_route_prefers_least_loaded_local`.
  - Gated dual-node smoke: `p08_remote_replica_generate_smoke` → worker CUDA load, coord routes, `output="Hi!"`.
  - Open inside P08: dynamic batching; concurrent two-loaded-replica proof (needs 2 model hosts).

# In progress
- Nothing mid-edit. Working tree clean on `39b82e4`. Next session starts at Next #1.

# Decisions
- Phases are serial for roadmap proof gates; Linux implementation may continue while Windows/Metal host proofs wait.
- Almost no code comments; prefer explicit names/types.
- `roadmap.md` is canonical; `checklist.md` tracks progress only.
- eframe/egui **0.36**: `App::ui(&mut Ui)`, `egui::Panel::{top,bottom}`.
- Prost build uses `protoc-bin-vendored`.
- Candle **0.11** with in-tree `qwen3`; complete-stage uses full `ModelForCausalLM`. True partial layer construct is **P09**.
- Runtime dtype: CUDA/Metal FP16, CPU F32.
- Complete-stage load needs **whole-shard** cache objects.
- Feature flags: `mesh-app --features cuda` → `mesh-node/cuda` → `mesh-inference/cuda` → `mesh-compute/cuda` (same for `metal`).
- P08 first cut: full-model replicas + control-plane remote generate. No activation frames (P09).
- Dynamic batching deferred (`FIRST_MAX_CONCURRENT_REQUESTS = 1`).
- Route order: least `active_requests`, then prefer local, then `node_id`.
- Remote generate: coordinator tokenizes; worker runs model + sampling; returns TokenResults (currently after generate completes, not mid-token live stream).
- Durable smoke cache: `$HOME/mesh-p07-smoke` (~7.6G).
- User-local CUDA 13.1: `$HOME/cuda-root` + `source $HOME/cuda-env.sh`. glibc 2.43 needs `rsqrt`/`rsqrtf` `noexcept` patch on toolkit `crt/math_functions.h` (`.bak` kept).
- Branch is **2 commits ahead of origin** (`c49ad72`, `39b82e4`); push when ready.

# Gotchas
- Host: NVIDIA driver + RTX 4070 SUPER; toolkit is user-local, not system `/usr/local/cuda`. Rebuilds need `source $HOME/cuda-env.sh`.
- Re-extracting CUDA debs into `$HOME/cuda-root` overwrites the rsqrt patch — re-apply from `.bak` / patch again.
- P08 dual-node smoke symlinks `model-cache` + `cache` from `$HOME/mesh-p07-smoke` into temp dirs.
- One 12GB GPU cannot hold two full Qwen3-4B FP16 replicas; concurrent two-replica proof needs a second host.
- Flow still: Probe → Prepare → (optional Load) → Generate. Remote route needs coordinator tokenizer via prepare (or prior load).
- Load fails if only range artifacts exist — re-prepare complete plan for whole shards.
- `UiCommand` is not `Eq` (`Generate` carries `f32`).
- Snapshot resets must preserve `models`, `resources`, `inference`.
- Qwen3 non-thinking template requires empty think block suffix.
- HF LFS digest is origin `x-linked-etag`, not CDN ETag after redirect.

# Next
1. **Default implementation next: P09 layer-pipeline** (partial stages, activation frame wire format, pipeline runtime, final-stage sampling). Read `docs/architecture/protocol/activation-frame.md` and inference parallelism docs first.
2. Optional P08 polish before P09: live per-token `TokenResult` while worker generates; dynamic batching after multi-slot engine.
3. When a second model-capable host exists: concurrent two-replica proof (checklist P08 proof box). Friend Windows LAN test is valid for this + Windows P07.5.
4. Host proofs when machines available: Windows CUDA / **P07.5**, macOS Metal. Do not block Linux P09 on them.
5. Push `c49ad72` + `39b82e4` when ready.

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

# GUI CUDA (reuses cache)
MESH_DATA_DIR=$HOME/mesh-p07-smoke cargo run -p mesh-app --release --features cuda
```

# Resume map
| Path | Role |
|---|---|
| `AGENTS.md` | Agent coding/phase rules |
| `docs/implementation/roadmap.md` | Canonical phase order |
| `docs/implementation/checklist.md` | Progress + P07/P08 evidence notes |
| `docs/architecture/protocol/activation-frame.md` | P09 wire format |
| `docs/architecture/inference/parallelism-and-edge-cases.md` | Replica vs pipeline rules |
| `docs/architecture/inference/tokenizer-and-sampling.md` | A05 |
| `docs/architecture/inference/kv-cache.md` | A06 |
| `crates/mesh-core/src/inference.rs` | Replica types + route helper |
| `crates/mesh-core/proto/mesh/v1/control.proto` | ReplicaStatus + inference msgs |
| `crates/mesh-net/src/inference.rs` | Control envelope builders |
| `crates/mesh-net/src/session.rs` | SessionCommand/Event inference path |
| `crates/mesh-inference/src/engine.rs` | load/generate_from_tokens/tokenizer |
| `crates/mesh-node/src/runtime.rs` | Replica announce/route/serve + smokes |
| `apps/mesh-app/src/app.rs` | Inference + peers UI |
| `$HOME/mesh-p07-smoke` | Durable prepare/load cache |
| `$HOME/cuda-root` + `$HOME/cuda-env.sh` | User-local CUDA 13.1 |
| `$HOME/cuda-debs` | Cached debs for toolkit re-extract |
