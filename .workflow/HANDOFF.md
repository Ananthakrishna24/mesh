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

# In progress
- The P01 workspace and native application shell are ready to implement.

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

# Next
1. Scaffold P01: root Cargo workspace, `mesh-core`, `mesh-store`, `mesh-net`, `mesh-node`, and `mesh-app`.
2. Pin initial dependency versions and target-gated features.
3. Open the eframe first-run window through `cargo run --release`.
4. Add typed GUI command and runtime snapshot channels.
5. Implement SQLite startup, migration zero-to-one, and atomic local identity creation.
6. Implement the manually reachable P02 Quinn enrollment path with generated Protobuf code.
7. Select router-mapping crates and peer-record merge rules before calling automatic internet enrollment complete.
8. Lock tokenizer, sampling, KV-cache, provider validation, and placement thresholds before their inference phases.
