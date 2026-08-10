# Goal
Design an extendable decentralized hardware mesh architecture for direct GPU compute across internet-connected PCs.

# Done
- Built the agent-readable `docs/` architecture knowledge base (`ec12b82`).
- Documented direct Quinn/QUIC peer connections (`ec12b82`).
- Documented distributed LLM inference and provider-backed partial model distribution (`6363c89`).
- Documented one-command native desktop onboarding and the complete roadmap (`31699c2`).
- Required native Windows and Linux NVIDIA CUDA plus macOS Apple Silicon Metal (`e3cd519`).
- Selected public dense Qwen3-4B for complete-model proofs and Qwen3-8B for distributed proof (`8bbfaf1`).
- Locked Protobuf control framing, protocol `1.0`, typed errors, and exact `HELLO`/`WELCOME` enrollment messages (`4582c8b`).
- Locked self-signed ECDSA P-256 QUIC identity, certificate-derived Node IDs, 30-minute one-time invitations, and exact text/file/URI encoding (`4582c8b`).
- Locked bundled SQLite state, transactional migrations, a dedicated storage worker, and native provider credential stores (`4582c8b`).
- Locked the 128-byte activation header, contiguous little-endian FP16 payload, 256 MiB limit, validation, and backpressure rules (`4582c8b`).
- Added `mesh-store` to the accepted crate boundaries and updated architecture indexes and roadmap (`4582c8b`).
- Added an exhaustive roadmap tracker at `docs/implementation/checklist.md` and indexed it from `docs/README.md` (uncommitted; roadmap anchor `31699c2`).
- Declared root `AGENTS.md` with serial phase order, almost-no-comments rule, crate-boundary, and checklist rules (uncommitted).
- Scaffolded the root Cargo workspace with `apps/mesh-app` and `crates/{mesh-core,mesh-store,mesh-net,mesh-hardware,mesh-model,mesh-compute,mesh-inference,mesh-node}` (uncommitted).
- Implemented typed `UiCommand` / `UiSnapshot` channels and the `mesh-node` runtime loop (uncommitted).
- Implemented the eframe first-run and empty dashboard screens in the `mesh` binary (uncommitted).
- Proved Linux P01 launch: `timeout 5s ./target/release/mesh` starts the native app and node runtime without a frontend build or helper process (uncommitted).
- Checked P01 build items and Linux proof evidence in `docs/implementation/checklist.md` (uncommitted).

# In progress
- P01 cross-platform proof is incomplete: Windows and macOS host launches remain open.
- P02 GUI-driven two-node enrollment has not started.

# Decisions
- Rust is the implementation language.
- Use Quinn QUIC over UDP with no public gateway, compute relay, permanent controller, or permanent master.
- Use one native `mesh` desktop application built with egui through eframe.
- Use Protobuf through Prost for control messages; frame each envelope with a four-byte big-endian length and limit it to 1 MiB.
- Protocol major mismatch rejects the connection; same-major peers negotiate the highest shared minor version.
- Use one persisted self-signed ECDSA P-256 certificate for client and server roles. `NodeId` is SHA-256 of certificate DER.
- Use Protobuf plus unpadded Base64 URL data for invitations: `mesh1:` text, `.mesh-invite` file, and `mesh://enroll/` URI.
- Use bundled SQLite through `rusqlite`; only `mesh-store` executes SQL. Use native keyring adapters for provider tokens with session-only fallback, never plaintext persistence.
- Use one QUIC unidirectional stream per activation: 128-byte fixed header plus contiguous little-endian FP16 bytes.
- Native Windows x64 and Linux x64 NVIDIA CUDA and macOS Apple Silicon Metal are required.
- Use Candle for the first Qwen3 proof, subject to native platform validation.
- Use Qwen3-4B as the normal development model and Qwen3-8B as the distributed acceptance model.
- Start Windows CI after the first confident native Windows Qwen3-4B CUDA implementation; manual Windows proofs remain required before then.
- Distributed training remains deferred.
- Keep `roadmap.md` canonical; `checklist.md` only tracks implementation progress, decision gates, proofs, and deferred status.
- Preserve A05 ownership choices as preferences until formally decided; do not turn preferred ownership into an accepted contract.
- Implement phases serially. Do not parallelize implementation phases.
- Write almost no code comments; prefer explicit names and types.
- P01 temporary IDs use UUID placeholders in `mesh-core`. Certificate-derived Node IDs replace them in P02.
- eframe/egui `0.36` uses `App::ui(&mut Ui)` and `egui::Panel::{top,bottom}` rather than the older `update` / `TopBottomPanel` API.

# Gotchas
- An inviter must accept an unknown joining certificate provisionally at TLS, but that connection may process only one bounded `HELLO` until its invitation binds and commits.
- Prost skips unknown fields and does not preserve them when re-encoding; peers must not decode and forward control messages.
- Independent QUIC activation streams may finish out of order. Transfer IDs prevent duplicates; request state and sequence position define execution order.
- The activation receiver validates identifiers, shape, checked byte counts, queue limits, and memory reservation before allocating the payload buffer.
- Qwen3-4B is approximately 8 GB and Qwen3-8B approximately 16.4 GB at 16-bit parameter storage before KV cache and runtime overhead.
- Candle's complete Qwen3 model builds every layer; the mesh must implement a stage-aware loader rather than loading and discarding unassigned layers.
- A short enrollment code cannot work without a public lookup service; the invitation carries endpoint details.
- Automatic enrollment cannot cross every CGNAT or firewall; the GUI provides guided recovery.
- The repository rule requires this handoff snapshot even though it is not project architecture documentation.
- `cargo run --release` is a long-lived GUI process; use a wall-clock timeout around the binary for smoke proofs.
- Closing the window must send `UiCommand::Shutdown` so the Tokio worker thread exits before process teardown.

# Next
1. Prove `cargo run --release` on Windows and macOS development hosts; check the remaining P01 proof boxes only after those runs.
2. Start P02: stable certificate-derived IDs, `mesh-store` SQLite identity transaction, Quinn endpoint, invitation create/input, `HELLO`/`WELCOME`, peer store, enrollment screens, restart reconnect.
3. Select router-mapping crates (A08) and peer-record merge rules (A10) before calling automatic internet enrollment complete.
4. Lock tokenizer/sampling (A05), KV-cache (A06), provider validation (A11-A13), and placement thresholds (A07) before their inference phases.
