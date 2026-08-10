# Goal
Design an extendable decentralized hardware mesh architecture for direct GPU compute across internet-connected PCs.

# Done
- Architecture docs and locked contracts through activation framing (`4582c8b`, `d34f0b4`).
- P01 workspace + native `mesh` shell, `AGENTS.md`, checklist (`706453a`).
- P02 enrollment store/runtime foundation (`bd9e28d`).
- A07 + P03 Linux hardware/network discovery committed (`73d2575`).
- A08 NAT/router mapping gate accepted (this session, uncommitted until user commits):
  - Contract: `docs/architecture/networking/nat-router-mapping.md`
  - ADR-0013: `docs/decisions/0013-nat-router-mapping-crates.md`
  - Crates: `igd-next` (`aio_tokio`) for UPnP; `crab_nat` for NAT-PMP + PCP
  - Quinn pre-bound socket path: `MeshEndpoint::from_udp_socket`
  - Proof test: `mesh-net::prebound_udp_socket_serves_quic`
- A10 peer-record merge gate accepted (this session, uncommitted until user commits):
  - Contract: `docs/architecture/networking/peer-record-merge.md`
  - ADR-0014: `docs/decisions/0014-peer-record-merge-rules.md`
  - `mesh-core` candidate/peer metadata + pure merge helpers + unit tests
  - `mesh-store` schema v2 peer columns + extended candidate JSON
- Indexes/checklist/roadmap updated so P04 prerequisites are checked.

# In progress
- Nothing mid-edit. Ready for P04 implementation.

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
- A08: PCP then NAT-PMP then UPnP; bind UDP socket first, map that port, Quinn `Endpoint::new` on same socket; no STUN/TURN/relay.
- A08 rejected `portmapper` hard dep (iroh-adjacent deps / toolchain).
- A10: per-field ownership merge; candidate lifetimes by kind; third-party capability bodies not authoritative in v1; enrolled peers retained offline; addresses expire instead.
- A10: `PeerUpdate` coalesce 5s on candidate churn; self refresh 10 min.

# Gotchas
- Quinn `SendStream::write_all` is inherent in this Quinn version.
- Do not put `0.0.0.0` in invitation candidates; normalize unspecified binds (`mesh-net/src/candidates.rs`).
- GUI close must send `UiCommand::Shutdown`.
- Isolated data dirs: `MESH_DATA_DIR=/tmp/mesh-a cargo run --release`.
- Metal discovery is feature-gated (`mesh-hardware` feature `metal`) and unproven on macOS.
- Windows NVML path compiles for Windows targets but is unproven on a Windows host.
- Full interactive two-window GUI hardware/benchmark proof remains manual; runtime path is unit-tested.
- Existing DBs on schema v1 migrate to v2 via `ALTER TABLE peers ...`; fresh DBs create v2 directly and set `user_version=2`.
- Wire `EndpointCandidate` still carries kind/address/priority only; local observation/expiry/reachability are filled on receive until a compatible proto extension is added in P04 if needed.

# Next
1. **Start P04 — Automatic direct connectivity** against A08/A10 contracts:
   - Add `igd-next` + `crab_nat` deps to `mesh-net`.
   - Implement router mapping module (PCP → NAT-PMP → UPnP) using pre-bound UDP socket.
   - Publish `RouterMapping` candidates with lease expiry.
   - Expand IPv6/IPv4 candidate collection.
   - Implement `PeerUpdate` apply path with `merge_peer_records` / `merge_candidates`.
   - Peer-assisted hole punching + guided firewall/manual forwarding recovery.
2. Optional manual GUI proof on Linux still useful anytime:
   ```bash
   MESH_DATA_DIR=/tmp/mesh-a cargo run --release
   MESH_DATA_DIR=/tmp/mesh-b cargo run --release
   ```
3. Before P07 inference: lock **A05** tokenizer/sampling, **A06** KV-cache, **A11–A13** provider/cache.
4. **End of project / final pass**: Windows and macOS `cargo run --release` + GUI enrollment/hardware proofs; then check remaining P01/P02/P03 proof boxes.

# Resume map
| Path | Role |
|---|---|
| `AGENTS.md` | Agent coding/phase rules |
| `docs/implementation/roadmap.md` | Canonical phase order |
| `docs/implementation/checklist.md` | Progress checkboxes |
| `docs/architecture/networking/nat-router-mapping.md` | A08 contract |
| `docs/architecture/networking/peer-record-merge.md` | A10 contract |
| `docs/architecture/networking/network-benchmark.md` | A07 contract |
| `crates/mesh-core/` | IDs, proto, invite, hardware/link/peer merge types, UI channel types |
| `crates/mesh-hardware/` | CPU/memory/disk/NVML/Metal discovery |
| `crates/mesh-store/` | SQLite only |
| `crates/mesh-net/` | Quinn, TLS, framing, handshake, benchmark, session, candidates |
| `crates/mesh-node/src/runtime.rs` | Runtime composition, enrollment, hardware, sessions |
| `apps/mesh-app/` | eframe GUI binary `mesh` |
