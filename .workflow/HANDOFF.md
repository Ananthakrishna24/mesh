# Goal
Design an extendable decentralized hardware mesh architecture for direct GPU compute across internet-connected PCs.

# Done
- Architecture docs and locked contracts through A11–A13; A05/A06 (`b3b4509`, `3e6e96a`).
- P01–P06 Linux paths (through `218e5de`).
- P07 single-node Linux CPU (`aa79a97`, `ebe19d8`) and CUDA (`c49ad72`) proofs.
- P08 full-model replica routing (`39b82e4`); P09 foundation, activation runtime, and multi-node QUIC path (`425bb5d`, `65febfb`).
- Model cache reuse, byte progress, sidecar retention, and remote-first equal-load routing (`6909540`).
- Added mesh-app two-PC pipeline controls: shared deployment ID, connected-peer selection, local First/Final role, runtime-derived layer placement, and central-panel scrolling (`8b6e48e`).
- Added two concurrent pipeline slots with request-scoped KV caches, request/transfer-ordered bounded queues, independent cancellation/cleanup, and runtime slot reporting (`e401ba6`).
- Verification for `e401ba6`: `cargo test --workspace` (63 passed); CUDA interleaved proof produced `A=[9707, 0, 2585]`, `B=[6033, 13, 151645]`; dual-node QUIC concurrent proof produced `"Hello! How can"` and `"Red."`.

# In progress
- Qwen3-8B distributed proof is blocked on a second directly connected model-capable PC. This workstation alone has a 12 GB RTX 4070 SUPER and 24.6 GB available RAM; co-locating both 8B FP16 stages exceeds VRAM and does not satisfy the two-PC proof.
- Working tree is clean before this handoff refresh.

# Decisions
- Pipeline stages support two concurrent requests; complete-model replicas remain at one request because `SingleNodeEngine` is moved into one blocking generation task.
- KV caches, sequence lengths, inbound activation queues, and transfer counters are request-scoped. Cancelling or finishing one request never clears another request's state.
- Inbound activations are buffered per request and released by transfer ID; stale, duplicate, over-capacity, and wrong sequence-position frames fail the request.
- `LoadPipelineStage` now accepts only deployment ID, local stage index, and ordered node IDs. Runtime derives repository, layer count, role, and layer range from the resolved manifest and `PlacementPlan::split_even`; redundant UI-supplied assignment data was removed.
- The two-PC UI uses the same deployment ID on both PCs. Choosing First orders `[local, peer]`; choosing Final orders `[peer, local]`, yielding the same placement on both machines.
- Do not claim the Qwen3-8B proof from a co-located simulation; the roadmap requires at least two directly connected PCs and ultimately a mixed-OS route.

# Gotchas
- CUDA commands require `source $HOME/cuda-env.sh`; the toolkit is under `$HOME/cuda-root`.
- The dual-node CUDA smoke takes about 6.5 minutes because each isolated runtime prepares and loads its stage.
- Only Qwen3-4B is cached under `$HOME/mesh-p07-smoke`; Qwen3-8B is not downloaded.
- No SSH peers are configured in this harness. Windows target, Windows CUDA host, and macOS Metal host are unavailable here.
- Both physical PCs must use the corrected dual-stack endpoint build and create a fresh invitation; old invitations may advertise unreachable candidates.
- Native mesh-app launch was smoke-tested with isolated `MESH_DATA_DIR`; the runtime started without panic, but Wayland/X11 window introspection did not expose the app window to `xwininfo`.

# Next
1. On two model-capable PCs, run the current build, select and probe Qwen3-8B, connect them, paste one shared deployment ID, and choose opposite First/Final roles.
2. Load both stages and confirm each UI reports the same deployment, complementary layer ranges, backend, and `0/2` available slots.
3. Start generation from both PCs concurrently; record stage load, activation transfer, token output, stop reason, and slot recovery to `0/2`.
4. Update the P09 checklist only after the real Qwen3-8B two-PC proof; retain the mixed-OS criterion until Windows/macOS hardware is available.
5. Optionally retry the friend-PC IPv6 route and run Windows CUDA/macOS Metal host proofs when those machines are reachable.
6. After P09 proof completion, continue P10 failure/restart behavior.
