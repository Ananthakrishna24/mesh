# Goal
Design an extendable decentralized hardware mesh architecture for direct GPU compute across internet-connected PCs.

# Done
- Architecture docs and locked contracts through activation framing (`4582c8b`, `d34f0b4`).
- P01 workspace + native `mesh` shell, `AGENTS.md`, checklist (`706453a`).
- P02 Linux enrollment (user committing separately if still dirty at session start).
- **A07 accepted**: network benchmark and placement cost contract + ADR-0012.
  - Canonical: `docs/architecture/networking/network-benchmark.md`
  - ADR: `docs/decisions/0012-network-benchmark-and-placement-cost.md`
- **P03 Linux implementation complete in working tree**:
  - `mesh-core` capability/link types, age/stability/threshold helpers, UI hardware/link views.
  - `mesh-hardware`: sysinfo CPU/memory/disk, NVML CUDA discovery (Linux/Windows feature), CPU FP32 proxy, Metal feature stub for macOS.
  - Control proto: CapabilityReport + BenchmarkRequest/Accept/Reject/Result.
  - `mesh-net` benchmark streams (`MSHB` header), session runner, post-handshake capability exchange, directional delay + bandwidth.
  - `mesh-node` discovers hardware on start, keeps sessions alive, publishes peer hardware lines and link metrics.
  - GUI dashboard shows local hardware, refresh, peer hardware, delay/bandwidth/stability.
- Verified on Linux:
  - `cargo test -p mesh-core --lib` — pass
  - `cargo test -p mesh-hardware --lib` — pass (NVML sees RTX 4070 SUPER)
  - `cargo test -p mesh-net --lib` — pass (`localhost_hello_welcome`, `localhost_capability_and_bandwidth`)
  - `cargo test -p mesh-node --lib` — pass (`two_nodes_enroll_over_localhost` waits for delay+bandwidth)
  - `cargo build --release -p mesh-app` — pass

# In progress
- Nothing mid-edit. P03 code is finished and uncommitted (plus any leftover P02 commit the user handles).

# Decisions
- Phases are serial. Do not parallelize implementation phases (`AGENTS.md`).
- Almost no code comments; prefer explicit names/types.
- `roadmap.md` is canonical; `checklist.md` tracks progress only.
- eframe/egui **0.36**: implement `App::ui(&mut Ui)`, use `egui::Panel::{top,bottom}`.
- Prost build uses `protoc-bin-vendored`.
- A07: half-RTT one-way estimate; 5/30 min fresh/stale/expired; pipeline hop rejects >80 ms / <10 Mbps / stability <50; max 3 WAN stages.
- Lower `NodeId` initiates post-handshake benchmarks so only one peer drives the sequence.
- Both peers send capability first; sessions drain the peer capability before starting benchmarks.
- Bandwidth uses unidirectional QUIC stream, default 4 MiB, max 16 MiB.
- First GPU token-rate remains deferred until real stage warm-up (P07); P03 only has CPU FP32 proxy + NVML inventory.

# Gotchas
- Commit A07+P03 before starting P04.
- Quinn `SendStream::write_all` is inherent in this Quinn version; `AsyncWriteExt` is not required for those writes.
- Do not put `0.0.0.0` in invitation candidates; normalize unspecified binds (`mesh-net/src/candidates.rs`).
- GUI close must send `UiCommand::Shutdown`.
- Isolated data dirs: `MESH_DATA_DIR=/tmp/mesh-a cargo run --release`.
- Metal discovery is feature-gated (`mesh-hardware` feature `metal`) and unproven on macOS.
- Windows NVML path is compiled for Windows targets but not proven on a Windows host.
- Full interactive two-window GUI hardware/benchmark proof remains manual; runtime path is covered by unit tests.

# Next
1. **Commit A07 + P03** (docs, proto, hardware, net session/benchmark, node, GUI, checklist/roadmap/handoff).
2. Optional manual GUI proof on Linux:
   ```bash
   MESH_DATA_DIR=/tmp/mesh-a cargo run --release
   # Create mesh → dashboard should show CPU/GPU
   MESH_DATA_DIR=/tmp/mesh-b cargo run --release
   # Enroll → both dashboards show peer hardware + delay/bandwidth
   ```
3. **Do not start P04** until **A08** (UPnP/NAT-PMP/PCP crates) and **A10** (peer-record merge rules) are resolved.
4. Before P07 inference: lock **A05** tokenizer/sampling, **A06** KV-cache, **A11–A13** provider/cache.
5. **End of project / final pass**: Windows and macOS `cargo run --release` + GUI enrollment/hardware proofs; then check remaining P01/P02/P03 proof boxes.

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
