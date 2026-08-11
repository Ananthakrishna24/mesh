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
- Verification for `e401ba6`: `cargo test --workspace` (63 passed); CUDA interleaved proof produced `A=[9707, 0, 2585]`, `B=[6033, 13, 151645]`; dual-node QUIC concurrent Qwen3-4B proof produced `"Hello! How can"` and `"Red."`.
- Prepared the exact physical two-PC Qwen3-8B test procedure in this handoff (`496482f` is the prior handoff anchor; code under test is `e401ba6` plus later documentation-only commits).

# In progress
- Run the physical Qwen3-8B layer-pipeline proof with a friend. This is the only unfinished P09 proof reachable next.
- Success requires two directly connected, model-capable PCs running the same revision. A Linux/Linux run proves the physical distributed path; a Windows/Linux or macOS/Linux run additionally advances the mixed-OS requirement.
- No Qwen3-8B artifact is cached on the local workstation yet. Expect a large first download on both PCs.

# Decisions
- Pipeline stages support two concurrent requests; complete-model replicas remain at one request because `SingleNodeEngine` is moved into one blocking generation task.
- KV caches, sequence lengths, inbound activation queues, and transfer counters are request-scoped. Cancelling or finishing one request never clears another request's state.
- Inbound activations are buffered per request and released by transfer ID; stale, duplicate, over-capacity, and wrong sequence-position frames fail the request.
- `LoadPipelineStage` accepts only deployment ID, local stage index, and ordered node IDs. Runtime derives repository, layer count, role, and layer range from the resolved manifest and `PlacementPlan::split_even`.
- Both PCs must use exactly the same deployment ID. Choosing First orders `[local, peer]`; choosing Final orders `[peer, local]`, so opposite choices produce the same placement on both PCs.
- For the stage proof, stop after **Probe / resolve**. Do not click **Prepare downloads**: that is the complete-model preparation path. **Load this PC's stage** performs the assignment-filtered download and load.
- Do not claim the Qwen3-8B proof from a co-located simulation. The roadmap requires at least two directly connected PCs and ultimately a mixed-OS route.

# Gotchas
- Both PCs need the corrected dual-stack endpoint build and a fresh invitation created after both updated builds are running. Never reuse an invitation from an older build.
- There is no relay. Direct WAN enrollment needs reachable global IPv6, successful router mapping/manual UDP forwarding, or another direct route. Two CGNAT-only networks may not connect; use the same LAN for a controlled proof if necessary.
- Allow the `mesh` executable through each OS firewall for inbound and outbound UDP. On Windows, accept the firewall prompt for the active network profile.
- Qwen3-8B is public; a Hugging Face token is normally optional. Save one in both apps if anonymous requests are rate-limited.
- Budget at least 25 GB free disk per PC and close GPU-heavy programs. Each machine should have roughly 10 GB free VRAM for its FP16 half-stage; lower-memory GPUs may fail during load.
- The local Linux CUDA environment requires `source $HOME/cuda-env.sh`; other Linux hosts may use their normal CUDA toolkit environment.
- Windows native CUDA and macOS Metal builds have not been compiled on this workstation. Windows needs Rust MSVC, Visual Studio C++ Build Tools, an NVIDIA driver, and a compatible CUDA toolkit. macOS requires Apple Silicon/Xcode tools and the `metal` feature.
- Qwen3-8B has 36 layers in the resolved configuration, so the expected two-way split is First `0..18` and Final `18..36`.
- Wait until both stages are Ready before generating. The current UI does not implement a cross-node ready barrier.
- Stage slot status should recover to `0/2`; fast generations may make intermediate `1/2` or `2/2` states difficult to catch visually. Terminal logs and final outputs are stronger evidence.

# Next
1. Put both PCs on the same code revision containing `e401ba6` and `8b6e48e`. Confirm with `git rev-parse --short HEAD`; documentation-only commits after `e401ba6` are fine, but both PCs must report the same hash.
2. Install platform prerequisites and start the native app from a terminal with logs:
   - Linux NVIDIA: `source $HOME/cuda-env.sh` only on the current workstation, then `RUST_LOG=info cargo run --release -p mesh-app --features cuda 2>&1 | tee mesh-p09-8b.log`.
   - Windows NVIDIA PowerShell: `$env:RUST_LOG="info"; cargo run --release -p mesh-app --features cuda 2>&1 | Tee-Object mesh-p09-8b.log`.
   - Apple Silicon macOS: `RUST_LOG=info cargo run --release -p mesh-app --features metal 2>&1 | tee mesh-p09-8b.log`.
3. On PC A, create/open the mesh and create a **fresh** invitation. On PC B, choose enrollment, paste the invitation, and connect. Confirm both dashboards list the other PC as connected before continuing.
4. If WAN enrollment fails, first allow UDP through both firewalls and create another fresh invitation. If both sides are behind CGNAT and have no global IPv6/router mapping, move the proof to the same LAN rather than treating the lack of a relay as a model failure.
5. On both PCs select **Qwen3-8B**, optionally save the same Hugging Face token, and click **Probe / resolve**. Compare the resolved repository, revision, and manifest identity on both PCs. They must match. Do **not** click **Prepare downloads**.
6. Expand **Two-PC pipeline placement** on both PCs. On PC A copy its deployment ID; paste that exact 32-hex-character value into PC B. Select the other PC in each peer selector.
7. Choose **First stage** on PC A and **Final stage** on PC B. Click **Load this PC's stage** on both PCs; parallel loading is acceptable. Keep both apps and terminal logs open during the download.
8. Wait for both loads to finish. Required state: same deployment ID; PC A reports stage 0 / first / layers `0..18`; PC B reports stage 1 / final / layers `18..36`; backend is `cuda` or `metal`; neither UI shows an error; slot status is available with maximum `2`.
9. Single-request check: from PC A enter `Say hello in one short sentence.`, use temperature `0`, max tokens `32`, seed `7`, and click **Generate**. Require non-empty streamed output, a normal stop reason, and no activation/deployment error. Repeat once from PC B with `Name one color.` and seed `11`.
10. Concurrency check: prepare different prompts on both PCs, set temperature `0` and max tokens `64`, count down, and click **Generate** on both within a few seconds. Both requests must complete with non-empty output. Neither may report `pipeline stage busy`, queue, deployment, sequence-position, or KV-cache errors. After completion, both stage views must return to `0/2`.
11. Cancellation isolation check: start a 128-token request on PC A, then start a second request on PC B. Cancel only PC A. PC A should stop as cancelled while PC B continues to a normal completion; both stages must recover to `0/2`.
12. Save proof artifacts: both `mesh-p09-8b.log` files; screenshots showing connected peers, identical deployment IDs, complementary stage ranges/backends, and final outputs; OS/GPU/VRAM details; resolved model revision/manifest; whether the route was WAN or same LAN.
13. On the next coding session, record the observed outputs/errors verbatim in `docs/implementation/checklist.md`. Mark the Qwen3-8B physical proof complete only if steps 8–10 pass. Mark the mixed-OS criterion complete only if the two PCs used different supported operating systems.
14. If the proof fails, preserve both logs and resume by identifying the first failing boundary: enrollment, resolve, assigned download, stage load, first activation, token feedback, concurrency, or cancellation. Do not begin P10 until the P09 failure is understood.
