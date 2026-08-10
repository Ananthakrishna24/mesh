# Goal
Design an extendable decentralized hardware mesh architecture for direct GPU compute across internet-connected PCs.

# Done
- Architecture docs and locked contracts through activation framing (`4582c8b`, `d34f0b4`).
- P01 workspace + native `mesh` shell, `AGENTS.md`, checklist (`706453a`).
- P02 enrollment store/runtime foundation (`bd9e28d`).
- A07 + P03 Linux hardware/network discovery committed (`73d2575`).
- A08 NAT/router mapping gate accepted earlier; implemented in P04.
- A10 peer-record merge gate accepted earlier; apply path wired in P04.
- **P04 automatic direct connectivity implemented this session (uncommitted until user commits):**
  - Deps: `igd-next` (`aio_tokio`), `crab_nat` on `mesh-net`.
  - `mesh-net/src/mapping.rs`: PCP → NAT-PMP → UPnP, pre-bound port, renew/delete, gateway discovery.
  - `mesh-net/src/candidates.rs`: IPv4/IPv6 collection, manual/router/peer-observed helpers.
  - `mesh-net/src/holepunch.rs` + proto `IntroductionOffer`/`IntroductionReady`/`PeerObserve`.
  - Session `PeerUpdate` + introduction command/event path.
  - `mesh-node` runtime: mapping during endpoint start, staggered dial, merge on `PeerUpdate`, hole-punch dials, guided recovery.
  - GUI recovery: retry automatic, manual UDP forwarding, firewall help.
  - Tests: `mesh-core` 7, `mesh-net` 9, `mesh-node` 1 all pass; `mesh-app` builds.

# In progress
- Nothing mid-edit. Ready for P05 or manual dual-NAT proof.

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
- P04: shared `MeshEndpoint` behind `Arc<Mutex<_>>` so accept loop and dials share one Quinn socket; reconnect/hole-punch sessions register `SessionCommand` senders via `PeerJoined`.
- P04 recovery primary action is always retry automatic; manual forwarding and firewall help are secondary/advanced.

# Gotchas
- Quinn `SendStream::write_all` is inherent in this Quinn version.
- Do not put `0.0.0.0` in invitation candidates; normalize unspecified binds (`mesh-net/src/candidates.rs`).
- GUI close must send `UiCommand::Shutdown`.
- Isolated data dirs: `MESH_DATA_DIR=/tmp/mesh-a cargo run --release`.
- Metal discovery is feature-gated (`mesh-hardware` feature `metal`) and unproven on macOS.
- Windows NVML path compiles for Windows targets but is unproven on a Windows host.
- Full interactive two-window GUI hardware/benchmark proof remains manual; runtime path is unit-tested.
- Existing DBs on schema v1 migrate to v2 via `ALTER TABLE peers ...`; fresh DBs create v2 directly and set `user_version=2`.
- Wire `EndpointCandidate` still carries kind/address/priority only; local observation/expiry/reachability are filled on receive.
- `crab_nat` NAT-PMP mappings need a separate `external_address` call for public IP; PCP returns external IP in the mapping type.
- Linux gateway discovery reads `/proc/net/route`; non-Linux falls back to guessing `.1` on the local /24.
- Failed join retry clears a half-created identity/endpoint before re-attempting.
- Real dual-NAT / CGNAT internet proof is still manual; localhost proves the available-path case only.

# Next
1. **Start P05 — Resource reservations** (offers, expiring leases, commit/release, concurrent coordinator conflicts, GUI state).
2. Optional manual dual-NAT / internet enrollment proof anytime:
   ```bash
   MESH_DATA_DIR=/tmp/mesh-a cargo run --release
   MESH_DATA_DIR=/tmp/mesh-b cargo run --release
   ```
3. Before P07 inference: lock **A05** tokenizer/sampling, **A06** KV-cache, **A11–A13** provider/cache.
4. **End of project / final pass**: Windows and macOS `cargo run --release` + GUI enrollment/hardware proofs; then check remaining P01/P02/P03/P04 proof boxes.

# Resume map
| Path | Role |
|---|---|
| `AGENTS.md` | Agent coding/phase rules |
| `docs/implementation/roadmap.md` | Canonical phase order |
| `docs/implementation/checklist.md` | Progress checkboxes |
| `docs/architecture/networking/nat-router-mapping.md` | A08 contract |
| `docs/architecture/networking/peer-record-merge.md` | A10 contract |
| `docs/architecture/networking/network-benchmark.md` | A07 contract |
| `docs/architecture/networking/direct-connection.md` | Dial/hole-punch flow |
| `crates/mesh-core/` | IDs, proto, invite, hardware/link/peer merge types, UI channel types |
| `crates/mesh-hardware/` | CPU/memory/disk/NVML/Metal discovery |
| `crates/mesh-store/` | SQLite only |
| `crates/mesh-net/` | Quinn, TLS, framing, handshake, benchmark, session, candidates, mapping, holepunch |
| `crates/mesh-node/src/runtime.rs` | Runtime composition, enrollment, hardware, sessions, recovery |
| `apps/mesh-app/` | eframe GUI binary `mesh` |
