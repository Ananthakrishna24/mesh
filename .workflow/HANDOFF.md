# Goal
Design an extendable decentralized hardware mesh architecture for direct GPU compute across internet-connected PCs.

# Done
- Architecture docs and locked contracts through A11–A13 (`4582c8b`, `d34f0b4`, `3e418a3`).
- P01 workspace + native `mesh` shell (`706453a`).
- P02 enrollment store/runtime foundation (`bd9e28d`).
- A07 + P03 Linux hardware/network discovery (`73d2575`).
- A08/A10 contracts + store peer schema v2 / merge helpers (`c4610c2` and prior).
- P04 automatic direct connectivity (`104816a`).
- P05 resource reservations (`ae8d345`).
- **P06 Linux implementation complete** (working tree may still be uncommitted — check `git status`):
  - `mesh-store`: keyring credentials (`mesh.model-provider.huggingface` / `default`), `model_cache_dir`, schema v4.
  - `mesh-model`: HF adapter (`hf-hub` 0.4 + explicit HTTP range), Qwen3 dense mapping, resolve/probe, range + complete-shard downloads, cache paths/publish/cleanup, prepare plans + disk netting, `FetchSource` trait.
  - `mesh-node`: Model Store runtime wiring (select/probe/prepare/cancel/cache/token + progress events).
  - `mesh-app`: dashboard Models card.
  - Checklist/roadmap current-state updated for P06 build items.
  - Verified: `cargo test -p mesh-core -p mesh-model -p mesh-store -p mesh-node --lib`; `cargo build -p mesh-app`.
  - Proofs: `prepare_uses_complete_shard_for_high_coverage`; `model_selection_updates_snapshot`.

# In progress
- Nothing actively mid-edit. Next session should commit P06 if still dirty, then lock A05/A06 (or optional Linux HF smoke first).

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
- **A11:** HF Hub only in v1; pin full 40-char commit SHA; `qwen3-dense@1.0.0`; manifest hash = SHA-256 of canonical JSON (sorted tensors/artifacts); cache key includes adapter/format/quant.
- **A12:** accept only validated `206`+`Content-Range`; `.partial` never worker-visible; bounded retries; complete-shard fallback on unsupported range or ≥80% coverage.
- **A13:** local `ProviderAccessReport`; disk prepare uses P05 leases net of cache hits; default `cache_max_bytes=0` unlimited soft cap + volume reserve floor `max(5GiB,5%)`; evict unreferenced only; 30 min partial grace.
- **P06 ownership:** provider/manifest/range logic in `mesh-model`; metadata/credentials in `mesh-store`; leases in `mesh-inference`.
- **P06 client stack:** `hf-hub 0.4` (tokio + rustls), explicit `reqwest` range path, `keyring 3` with `linux-native` (keyutils) — avoids dbus-dev on Linux.
- Credential save failure → session-only token, surfaced truthfully.
- Download engine uses `FetchSource` so unit tests fixture prepare without Hub.
- **Windows is not parallel.** No Windows implementation track now. Windows earnest work starts at **P07.5** (manual GUI/enrollment/download/CUDA generate, then CI). End-of-project closes remaining Win/macOS P01–P05 host proofs. Multi-host P06 Qwen3-8B download proof needs real hosts later; do not block A05/A06/P07 on it.

# Gotchas
- Quinn `SendStream::write_all` is inherent in this Quinn version.
- Do not put `0.0.0.0` in invitation candidates; normalize unspecified binds (`mesh-net/src/candidates.rs`).
- GUI close must send `UiCommand::Shutdown`.
- Isolated data dirs: `MESH_DATA_DIR=/tmp/mesh-a cargo run --release`.
- Metal discovery feature-gated (`mesh-hardware` `metal`); unproven on macOS.
- Windows NVML path compiles but is unproven on a Windows host.
- Full interactive two-window GUI proof remains manual.
- DBs schema v1 → v2 peers; v2 → v3 reservations; **v3 → v4 model_manifests + model_cache_entries**. Fresh DBs set `user_version=4`.
- Wire `EndpointCandidate` is still kind/address/priority only; local observation/expiry/reachability filled on receive.
- `crab_nat` NAT-PMP needs separate `external_address` for public IP; PCP carries external IP in mapping type.
- Linux gateway: `/proc/net/route`; non-Linux guesses `.1` on local /24.
- Failed join retry clears half-created identity/endpoint before re-attempt.
- Real dual-NAT/CGNAT internet proof is manual; localhost covers available-path only.
- `run_connected_session` now takes 8 args (events + `SessionCommand` rx).
- `ErrorMessage` now has optional `deployment_id`/`request_id`/`transfer_id` fields; constructors must set them.
- Three-node simultaneous enrollment over real WAN candidates is flaky in unit tests; keep multi-coordinator proofs on localhost/local manager paths for CI.
- `reserve_on_peer` exists for later coordinator use but is unused in P05 UI; allowed dead_code for now.
- `UiSnapshot` includes `models: ModelStoreView` with progress/error/busy fields; update any manual struct literals.
- Canonical manifest hashing currently uses sorted JSON (`serde_json`); CBOR can replace later if needed but hash inputs must stay stable once deployments pin hashes.
- `keyring` default secret-service feature needs `libdbus-1-dev`; workspace pins `linux-native` instead.
- Live HF resolve/prepare needs network; CI proofs stay fixture-based. Manual Linux: Select Qwen3-4B → Check access → Probe/resolve → Prepare downloads.
- Model cache root is `data_dir/model-cache` (sibling of `mesh.db`); HF client cache is under `cache_dir/hf-hub`.
- First-run / cancel-enrollment snapshot resets must preserve `models` and `resources` views.
- P06 multi-platform download proof boxes stay unchecked until real multi-host runs.

# Next
1. **Commit** P06 working tree if `git status` is dirty (docs + code together is fine).
2. Default product path: lock **A05** tokenizer/sampling ownership, then **A06** KV-cache contract (both required before P07).
3. Optional Linux P06 smoke (network):
   ```bash
   MESH_DATA_DIR=/tmp/mesh-a cargo run --release
   ```
   Create mesh → Models: Select Qwen3-4B → Check access → Probe/resolve → Prepare downloads.
4. Optional anytime: dual-window enrollment:
   ```bash
   MESH_DATA_DIR=/tmp/mesh-a cargo run --release
   MESH_DATA_DIR=/tmp/mesh-b cargo run --release
   ```
5. After A05/A06: start **P07** single-node Qwen3-4B (Linux CUDA first).
6. **P07.5** is when Windows work starts. End of project: remaining Win/macOS host proofs.

# Resume map
| Path | Role |
|---|---|
| `AGENTS.md` | Agent coding/phase rules |
| `docs/implementation/roadmap.md` | Canonical phase order |
| `docs/implementation/checklist.md` | Progress checkboxes |
| `docs/architecture/inference/model-distribution.md` | A11–A13 contract |
| `docs/architecture/inference/qwen3-model-family.md` | Family + stage ownership (A01/P07) |
| `docs/decisions/0015-provider-manifest-download-cache.md` | A11–A13 ADR |
| `crates/mesh-model/src/huggingface.rs` | HF resolve/probe/range/fetch |
| `crates/mesh-model/src/download.rs` | Prepare plans + downloads |
| `crates/mesh-model/src/adapter.rs` | Qwen3 dense mapping |
| `crates/mesh-model/src/cache.rs` | Cache paths/cleanup |
| `crates/mesh-store/src/credentials.rs` | HF token keyring adapter |
| `crates/mesh-node/src/runtime.rs` | Model Store wiring |
| `apps/mesh-app/src/app.rs` | Models dashboard card |
