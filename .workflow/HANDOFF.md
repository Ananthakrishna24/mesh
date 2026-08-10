# Goal
Design an extendable decentralized hardware mesh architecture for direct GPU compute across internet-connected PCs.

# Done
- Architecture docs and locked contracts through activation framing (`4582c8b`, `d34f0b4`).
- P01 workspace + native `mesh` shell, `AGENTS.md`, checklist (`706453a`).
- P02 enrollment store/runtime foundation (`bd9e28d`).
- A07 + P03 Linux hardware/network discovery committed (`73d2575`):
  - A07 contract + ADR-0012 (`docs/architecture/networking/network-benchmark.md`, `docs/decisions/0012-network-benchmark-and-placement-cost.md`).
  - `mesh-core` capability/link types, age/stability/threshold helpers, UI hardware/link views.
  - `mesh-hardware`: sysinfo CPU/memory/disk, NVML CUDA discovery, CPU FP32 proxy, Metal feature path.
  - Control proto: CapabilityReport + BenchmarkRequest/Accept/Reject/Result.
  - `mesh-net` benchmark streams (`MSHB`), session runner, post-handshake capability + directional delay/bandwidth.
  - `mesh-node` hardware on start, live sessions, peer hardware/link metrics.
  - GUI dashboard: local hardware, refresh, peer hardware, delay/bandwidth/stability.
- Verified on Linux at commit time:
  - `cargo test -p mesh-core -p mesh-hardware -p mesh-net -p mesh-node --lib` — pass
  - `cargo build --release -p mesh-app` — pass

# In progress
- Nothing mid-edit. Working tree clean after `73d2575` (handoff snapshot refresh only if still dirty).

# Decisions
- Phases are serial. Do not parallelize implementation phases (`AGENTS.md`).
- Almost no code comments; prefer explicit names/types.
- `roadmap.md` is canonical; `checklist.md` tracks progress only.
- eframe/egui **0.36**: implement `App::ui(&mut Ui)`, use `egui::Panel::{top,bottom}`.
- Prost build uses `protoc-bin-vendored`.
- A07: half-RTT one-way estimate; 5/30 min fresh/stale/expired; pipeline hop rejects >80 ms / <10 Mbps / stability <50; max 3 WAN stages.
- Lower `NodeId` initiates post-handshake benchmarks.
- Both peers send capability first; sessions drain peer capability before benchmarks.
- Bandwidth uses unidirectional QUIC stream, default 4 MiB, max 16 MiB.
- First GPU token-rate deferred until real stage warm-up (P07).

# Gotchas
- Quinn `SendStream::write_all` is inherent in this Quinn version.
- Do not put `0.0.0.0` in invitation candidates; normalize unspecified binds (`mesh-net/src/candidates.rs`).
- GUI close must send `UiCommand::Shutdown`.
- Isolated data dirs: `MESH_DATA_DIR=/tmp/mesh-a cargo run --release`.
- Metal discovery is feature-gated (`mesh-hardware` feature `metal`) and unproven on macOS.
- Windows NVML path compiles for Windows targets but is unproven on a Windows host.
- Full interactive two-window GUI hardware/benchmark proof remains manual; runtime path is unit-tested.

# Next
1. Optional manual GUI proof on Linux:
   ```bash
   MESH_DATA_DIR=/tmp/mesh-a cargo run --release
   # Create mesh → dashboard should show CPU/GPU
   MESH_DATA_DIR=/tmp/mesh-b cargo run --release
   # Enroll → both dashboards show peer hardware + delay/bandwidth
   ```
2. **Do not start P04** until **A08** (UPnP/NAT-PMP/PCP crates) and **A10** (peer-record merge rules) are resolved.
3. Before P07 inference: lock **A05** tokenizer/sampling, **A06** KV-cache, **A11–A13** provider/cache.
4. **End of project / final pass**: Windows and macOS `cargo run --release` + GUI enrollment/hardware proofs; then check remaining P01/P02/P03 proof boxes.

# Resume map
| Path | Role |
|---|---|
| `AGENTS.md` | Agent coding/phase rules |
| `docs/implementation/roadmap.md` | Canonical phase order |
| `docs/implementation/checklist.md` | Progress checkboxes |
| `docs/architecture/networking/network-benchmark.md` | A07 contract |
| `crates/mesh-core/` | IDs, proto, invite, hardware/link types, UI channel types |
| `crates/mesh-hardware/` | CPU/memory/disk/NVML/Metal discovery |
| `crates/mesh-store/` | SQLite only |
| `crates/mesh-net/` | Quinn, TLS, framing, handshake, benchmark, session |
| `crates/mesh-node/src/runtime.rs` | Runtime composition, enrollment, hardware, sessions |
| `apps/mesh-app/` | eframe GUI binary `mesh` |
