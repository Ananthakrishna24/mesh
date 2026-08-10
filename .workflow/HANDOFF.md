# Goal
Design an extendable decentralized hardware mesh architecture for direct GPU compute across internet-connected PCs.

# Done
- Architecture docs and locked contracts through activation framing (`4582c8b`, `d34f0b4`).
- P01 workspace + native `mesh` shell, `AGENTS.md`, checklist (`706453a`).
- P02 enrollment store/runtime foundation (`bd9e28d`).
- A07 + P03 Linux hardware/network discovery (`73d2575`).
- A08/A10 contracts + store peer schema v2 / merge helpers (`c4610c2` and prior).
- P04 automatic direct connectivity (`104816a`).
- P05 resource reservations (`ae8d345`).
- **A11–A13 locked** this session (docs ready to commit with P06 foundation):
  - Expanded `docs/architecture/inference/model-distribution.md` with full A11/A12/A13 contracts.
  - ADR-0015 provider manifest, partial download, and cache policy.
  - Roadmap/checklist/decisions index/docs index updated; A11–A13 checkboxes resolved.
- **P06 foundation started** (same change set, uncommitted):
  - `mesh-core`: model identity/reference/access/cache view types; `UiSnapshot.models`.
  - `mesh-model`: Safetensors header parser, range merge, Content-Range validation, canonical manifest hash.
  - `mesh-store` schema v4: `model_manifests` + `model_cache_entries` repos/APIs + unit proof.
  - Verified: `cargo test -p mesh-core -p mesh-model -p mesh-store -p mesh-node --lib`; `cargo build -p mesh-app`.

# In progress
- P06 model provider and cache implementation after foundation.
- User should commit A11–A13 docs + P06 foundation together (or split docs-first if preferred).

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
- P05 proof: exclusive local capacity via manager unit test + runtime probe path.
- **A11:** HF Hub only in v1; pin full 40-char commit SHA; `qwen3-dense@1.0.0`; manifest hash = SHA-256 of canonical JSON (sorted tensors/artifacts); cache key includes adapter/format/quant.
- **A12:** accept only validated `206`+`Content-Range`; `.partial` never worker-visible; bounded retries inside reservation lease; complete-shard fallback on unsupported range or ≥80% coverage.
- **A13:** local `ProviderAccessReport`; disk prepare uses P05 leases net of cache hits; default `cache_max_bytes=0` unlimited soft cap + volume reserve floor `max(5GiB,5%)`; evict unreferenced only; 30 min partial grace.
- **P06 ownership:** provider/manifest/range logic in `mesh-model`; metadata/credentials in `mesh-store`; leases in `mesh-inference`.

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
- `UiSnapshot` now includes `models: ModelStoreView`; update any manual struct literals if added later.
- Canonical manifest hashing currently uses sorted JSON (`serde_json`); CBOR can replace later if needed but hash inputs must stay stable once deployments pin hashes.
- HF adapter / live downloads not implemented yet; no network provider calls in this foundation slice.

# Next
1. Continue **P06**:
   - Hugging Face adapter (`hf-hub` + explicit HTTP range path).
   - Immutable revision resolve + access probe.
   - Range download + complete-shard fallback + partial resume.
   - Wire Model Store into `mesh-node` runtime and GUI (selection, token save/delete, progress, failures).
   - Parallel prepare plan types and disk-reservation netting from cache hits.
2. Optional anytime: dual-NAT / two-window GUI enrollment proof:
   ```bash
   MESH_DATA_DIR=/tmp/mesh-a cargo run --release
   MESH_DATA_DIR=/tmp/mesh-b cargo run --release
   ```
3. Before P07 inference: lock **A05** tokenizer/sampling and **A06** KV-cache.
4. **End of project**: Windows and macOS `cargo run --release` + GUI enrollment/hardware proofs; check remaining P01–P05 host proof boxes.

# Resume map
| Path | Role |
|---|---|
| `AGENTS.md` | Agent coding/phase rules |
| `docs/implementation/roadmap.md` | Canonical phase order (P06 in progress) |
| `docs/implementation/checklist.md` | Progress checkboxes |
| `docs/architecture/inference/model-distribution.md` | A11–A13 canonical contract |
| `docs/decisions/0015-provider-manifest-download-cache.md` | A11–A13 ADR |
| `docs/architecture/system/persistent-state.md` | Durable manifests/cache + schema v4 note |
| `crates/mesh-core/src/model.rs` | Model identity/access/cache types |
| `crates/mesh-model/` | Parser, validation, manifest hash |
| `crates/mesh-store/src/repos/models.rs` | Manifest/cache SQLite repos |
| `crates/mesh-node/src/runtime.rs` | Runtime composition (Model Store wiring next) |
| `apps/mesh-app/` | eframe GUI binary `mesh` |
