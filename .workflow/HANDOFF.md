# Goal
Design an extendable decentralized hardware mesh architecture for direct GPU compute across internet-connected PCs.

# Done
- Architecture docs and locked contracts through activation framing (`4582c8b`, `d34f0b4`).
- P01 workspace + native `mesh` shell, `AGENTS.md`, checklist (`706453a`).
- P02 enrollment store/runtime foundation (`bd9e28d`).
- A07 + P03 Linux hardware/network discovery (`73d2575`).
- A08/A10 contracts + store peer schema v2 / merge helpers (`c4610c2` and prior).
- P04 automatic direct connectivity (`104816a`).
- **P05 resource reservations** — ready to commit this session:
  - Control proto: `ResourceQuery`/`Offer`/`Reserve*`/`Commit`/`Release` on fields 20–26; introduction messages moved to 50–52 to match canonical control-protocol numbering.
  - `mesh-core`: `DeploymentId`, `ReservationId`, `resource.rs` amounts/leases/views; UI snapshot `resources` + probe/release commands.
  - `mesh-inference::LocalResourceManager`: offer, reserve, commit, release, expiry, owner cleanup; exclusive-capacity unit proof.
  - `mesh-store` schema v3 `reservations` table + repo CRUD; restore on runtime start.
  - `mesh-net` reservation wire helpers + session command/event handlers.
  - `mesh-node` runtime: manager wiring, sweep tick, remote request handling, local probe path, GUI state publish.
  - GUI dashboard **Local resources** card with probe/release.
  - Checklist/roadmap updated; P05 build + proof checked.
  - Verified: `cargo test -p mesh-core -p mesh-inference -p mesh-store -p mesh-net -p mesh-node --lib`; `cargo build -p mesh-app`.
  - After commit: replace this bullet’s “ready to commit” with the new hash.

# In progress
- Nothing mid-edit. User committing P05, then next session starts **P06** after A11–A13.

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
- A08: PCP → NAT-PMP → UPnP; bind UDP socket first, map that port, Quinn on same socket; no STUN/TURN/relay.
- A08 rejected `portmapper` hard dep.
- A10: per-field ownership merge; candidate lifetimes by kind; third-party capability not authoritative in v1; enrolled peers retained offline; addresses expire.
- A10: `PeerUpdate` coalesce 5s; self refresh 10 min.
- P04: `MeshEndpoint` shared via `Arc<Mutex<_>>`; reconnect/hole-punch register `SessionCommand` via `PeerJoined.session_commands`.
- P04 recovery primary = retry automatic; manual forwarding + firewall help are advanced.
- P05: Local Resource Manager lives in `mesh-inference` (workspace ownership), not `mesh-compute`.
- P05: default hold lease 60s, commit lease 30 min, max 2 h; offer TTL 15s.
- P05: introduction control fields renumbered to 50–52 so resource messages keep canonical 20–26.
- P05 proof: exclusive local capacity via manager unit test + runtime probe path; remote wire path is implemented and exercised by session handlers (full multi-coordinator internet enrollment remains separate from this phase proof).

# Gotchas
- Quinn `SendStream::write_all` is inherent in this Quinn version.
- Do not put `0.0.0.0` in invitation candidates; normalize unspecified binds (`mesh-net/src/candidates.rs`).
- GUI close must send `UiCommand::Shutdown`.
- Isolated data dirs: `MESH_DATA_DIR=/tmp/mesh-a cargo run --release`.
- Metal discovery feature-gated (`mesh-hardware` `metal`); unproven on macOS.
- Windows NVML path compiles but is unproven on a Windows host.
- Full interactive two-window GUI proof remains manual.
- DBs schema v1 → v2 peers; v2 → v3 adds `reservations`. Fresh DBs set `user_version=3`.
- Wire `EndpointCandidate` is still kind/address/priority only; local observation/expiry/reachability filled on receive.
- `crab_nat` NAT-PMP needs separate `external_address` for public IP; PCP carries external IP in mapping type.
- Linux gateway: `/proc/net/route`; non-Linux guesses `.1` on local /24.
- Failed join retry clears half-created identity/endpoint before re-attempt.
- Real dual-NAT/CGNAT internet proof is manual; localhost covers available-path only.
- `run_connected_session` now takes 8 args (events + `SessionCommand` rx).
- `ErrorMessage` now has optional `deployment_id`/`request_id`/`transfer_id` fields; constructors must set them.
- Three-node simultaneous enrollment over real WAN candidates is flaky in unit tests; keep multi-coordinator proofs on localhost/local manager paths for CI.
- `reserve_on_peer` exists for later coordinator use but is unused in P05 UI; allowed dead_code for now.

# Next
1. **Lock A11–A13** before P06:
   - A11 provider manifest generation (HF Hub first).
   - A12 partial download validation.
   - A13 provider access + local cache policy (disk reservation already partially overlaps P05 leases).
2. **Start P06 — Model provider and cache** against roadmap/checklist after those gates.
3. Optional anytime: dual-NAT / two-window GUI enrollment proof:
   ```bash
   MESH_DATA_DIR=/tmp/mesh-a cargo run --release
   MESH_DATA_DIR=/tmp/mesh-b cargo run --release
   ```
4. Before P07 inference: also lock **A05** tokenizer/sampling and **A06** KV-cache.
5. **End of project**: Windows and macOS `cargo run --release` + GUI enrollment/hardware proofs; check remaining P01–P05 host proof boxes.

# Resume map
| Path | Role |
|---|---|
| `AGENTS.md` | Agent coding/phase rules |
| `docs/implementation/roadmap.md` | Canonical phase order (P06 next after A11–A13) |
| `docs/implementation/checklist.md` | Progress checkboxes |
| `docs/architecture/protocol/control-protocol.md` | Control envelope + reservation message names |
| `docs/architecture/inference/README.md` | Reservation protocol flow |
| `docs/architecture/system/node-modules.md` | M09 Local Resource Manager |
| `docs/architecture/system/persistent-state.md` | Durable reservations |
| `crates/mesh-core/` | IDs, proto, resource types, UI types |
| `crates/mesh-inference/` | `LocalResourceManager` |
| `crates/mesh-store/` | SQLite + reservations repo |
| `crates/mesh-net/` | Quinn, reservation wire, session |
| `crates/mesh-node/src/runtime.rs` | Runtime composition |
| `apps/mesh-app/` | eframe GUI binary `mesh` |
