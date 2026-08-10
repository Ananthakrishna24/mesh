# Goal
Design an extendable decentralized hardware mesh architecture for direct GPU compute across internet-connected PCs.

# Done
- Architecture docs and locked contracts through A11–A13; A05/A06.
- P01–P06 Linux paths.
- P07 Linux CPU + CUDA host proofs (Qwen3-4B prepare/load/generate).
- **P08 replica inference implementation** (this session):
  - Proto `ReplicaStatus` + wire builders in `mesh-net`.
  - Session events/commands for ReplicaStatus, InferenceRequest, TokenResult, CancelRequest.
  - Replica registry on peers; least-loaded route (`select_replica_route`).
  - Local concurrency slots (`FIRST_MAX_CONCURRENT_REQUESTS`); load/health advertise on load/generate.
  - Coordinator tokenizer for remote routing without local engine weights.
  - GUI: replicas list, routed node, peer replica line, generate if any replica ready.
  - Unit: `select_replica_route_prefers_least_loaded_local`.
  - Gated smoke: `p08_remote_replica_generate_smoke` — worker CUDA load, coord routes remote generate `output="Hi!"`.
  - Dynamic batching **not** implemented; multi-replica concurrent proof still open.

# In progress
- Nothing mid-edit.

# Decisions
- Phases stay serial for roadmap proof gates, but implementation may proceed while Windows/Metal host proofs wait.
- P08 first cut: full-model replicas + remote request routing over control plane (token stream as TokenResult messages). No activation frames (those are P09).
- Dynamic batching deferred (still `FIRST_MAX_CONCURRENT_REQUESTS = 1`).
- Remote generate requires coordinator-side tokenizer (from prepare/load sidecars); worker owns model weights + sampling.
- Route preference: least `active_requests`, then prefer local, then node_id.
- Windows/Metal proofs remain manual when hosts exist; do not block further Linux implementation.

# Gotchas
- Dual-node P08 smoke symlinks `model-cache` + `cache` from `$HOME/mesh-p07-smoke` into temp dirs.
- One 12GB GPU: loading two full Qwen3-4B CUDA replicas on one host is not practical; remote smoke loads model on worker only.
- CUDA builds need `source $HOME/cuda-env.sh` (user-local toolkit + rsqrt noexcept patch).
- Control-plane token streaming is coarse (batch TokenResult after generate for remote-served requests); mid-token live stream to owner can be tightened later.
- `UiCommand::Generate` still one in-flight generation on the coordinator UI (`busy` gate).

# Next
1. Optional: true concurrent two-replica proof when a second model-capable host exists (friend Windows or second Linux GPU box).
2. P07.5 / Windows CUDA host proof when Windows PC available.
3. macOS Metal host proof when Mac available.
4. P09 layer-pipeline (partial stages, activation frames) when ready to start distributed model split.
5. Optional polish: stream TokenResult per token from worker while generating; dynamic batching after multi-slot engine.
6. Push commits when ready.

# Resume map
| Path | Role |
|---|---|
| `crates/mesh-core/src/inference.rs` | ReplicaEndpointView, select_replica_route |
| `crates/mesh-core/proto/mesh/v1/control.proto` | ReplicaStatus + InferenceRequest/TokenResult |
| `crates/mesh-net/src/inference.rs` | Control envelope builders |
| `crates/mesh-net/src/session.rs` | SessionCommand/Event for inference |
| `crates/mesh-inference/src/engine.rs` | generate_from_tokens, load_mesh_tokenizer |
| `crates/mesh-node/src/runtime.rs` | Replica announce/route/remote serve + P08 smoke |
| `apps/mesh-app/src/app.rs` | Replicas UI |
| `$HOME/mesh-p07-smoke` | Shared prepare cache |
| `$HOME/cuda-root` + `$HOME/cuda-env.sh` | CUDA toolkit |
