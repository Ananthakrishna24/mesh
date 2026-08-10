# Goal
Design an extendable decentralized hardware mesh architecture for direct GPU compute across internet-connected PCs.

# Done
- Architecture docs and locked contracts through activation framing (`4582c8b`, `d34f0b4`).
- P01 workspace + native `mesh` shell, `AGENTS.md`, checklist (`706453a`).
- P02 Linux enrollment implementation is complete in the working tree but **not committed**:
  - Certificate-derived `NodeId` (SHA-256 of cert DER), random 16-byte `MeshId`.
  - `mesh-store` SQLite: identity, peers, invitations, onboarding.
  - Quinn endpoint, mutual TLS, custom mesh cert verification.
  - Control proto in `crates/mesh-core/proto/mesh/v1/control.proto` via Prost + vendored `protoc`.
  - `mesh1:` invitation encode/decode (Protobuf + unpadded Base64 URL).
  - HELLO/WELCOME with 4-byte big-endian length framing.
  - Peer Store bind/consume invitation transaction.
  - GUI: create mesh, add PC / copy invite, enroll paste, progress, dashboard peers.
  - Restart path loads identity/peers and spawns reconnect.
- Verified on Linux (this machine):
  - `cargo test -p mesh-net localhost_hello_welcome` — pass
  - `cargo test -p mesh-node two_nodes_enroll_over_localhost` — pass
  - `cargo build --release` — pass
- Checklist updated: P01 build + Linux proof; P02 build + Linux proof. Windows/macOS proofs left open on purpose.

# In progress
- Nothing actively mid-edit. P02 code is finished and uncommitted.
- Windows and macOS P01/P02 proofs deferred to the end (user request).

# Decisions
- Phases are serial. Do not parallelize implementation phases (`AGENTS.md`).
- Almost no code comments; prefer explicit names/types.
- `roadmap.md` is canonical; `checklist.md` tracks progress only.
- eframe/egui **0.36**: implement `App::ui(&mut Ui)`, use `egui::Panel::{top,bottom}` (not old `update` / `TopBottomPanel`).
- Prost build uses `protoc-bin-vendored` (no system `protoc` required).
- Invitation TTL 30 minutes; inviter clock is authoritative.
- SQLite is single-writer on the runtime task; accept loop only forwards `IncomingPeer` events.
- Keep QUIC connections alive briefly after handshake so WELCOME is not dropped.
- Isolated data dirs for multi-instance: `MESH_DATA_DIR=/tmp/mesh-a cargo run --release`.
- Temporary P01 UUID NodeIds are gone; P02 uses cert-derived IDs.

# Gotchas
- Uncommitted P02 surface is large: modified P01 crates plus new net/store/core files. Commit before starting P03.
- Quinn `SendStream::flush` needs `use tokio::io::AsyncWriteExt`.
- Do not put `0.0.0.0` in invitation candidates; normalize unspecified binds to localhost / real local IPs (`mesh-net/src/candidates.rs`).
- Peer cert: `connection.peer_identity()` downcasts to `Vec<CertificateDer<'static>>`.
- Inviter TLS must accept unknown joining certs provisionally; only HELLO + valid invite may commit durable peer state.
- GUI close must send `UiCommand::Shutdown` so the Tokio worker thread exits.
- `cargo run --release` is long-lived; smoke with `timeout` or kill the window.
- Full interactive two-window GUI enrollment was not manually clicked end-to-end; the runtime path is proven by the multi-thread unit test using the same commands.

# Next
1. **Commit P02** (working tree is dirty). Suggested scope: enrollment identity/store/net/GUI + checklist/roadmap/handoff.
2. Optional manual GUI proof on Linux:
   ```bash
   MESH_DATA_DIR=/tmp/mesh-a cargo run --release
   # Create mesh → Add another PC → copy invite
   MESH_DATA_DIR=/tmp/mesh-b cargo run --release
   # Enroll this PC → paste invite
   ```
3. **Do not start P03** until **A07** (network benchmark and placement cost) is resolved — checklist gate.
4. Before automatic internet enrollment is “complete”: resolve **A08** (UPnP/NAT-PMP/PCP crates) and **A10** (peer-record merge rules). That unlocks finishing P04.
5. Before P07 inference: lock **A05** tokenizer/sampling, **A06** KV-cache, **A11–A13** provider/cache.
6. **End of project / final pass**: Windows and macOS `cargo run --release` + GUI enrollment proofs; then check remaining P01/P02 proof boxes.

# Resume map
| Path | Role |
|---|---|
| `AGENTS.md` | Agent coding/phase rules |
| `docs/implementation/roadmap.md` | Canonical phase order |
| `docs/implementation/checklist.md` | Progress checkboxes |
| `crates/mesh-core/` | IDs, proto, invite, UI channel types |
| `crates/mesh-store/` | SQLite only |
| `crates/mesh-net/` | Quinn, TLS, framing, handshake |
| `crates/mesh-node/src/runtime.rs` | Runtime composition, enrollment orchestration |
| `apps/mesh-app/` | eframe GUI binary `mesh` |
