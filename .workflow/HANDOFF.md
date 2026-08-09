# Goal
Design an extendable decentralized hardware mesh architecture for direct GPU compute across internet-connected PCs.

# Done
- Built the agent-readable `docs/` architecture knowledge base (`ec12b82`).
- Documented direct Quinn/QUIC peer connections (`ec12b82`).
- Documented distributed LLM inference, placement, reservations, batching, pipeline execution, and failures (`6363c89`).
- Documented provider-backed partial model distribution and synchronization (`6363c89`).
- Documented one-command native Rust desktop onboarding, enrollment data, automatic setup, guided failures, and the dashboard boundary (`31699c2`).
- Added the canonical remaining-decision and implementation roadmap (`31699c2`).

# In progress
- Architecture discussion continues before Rust scaffolding begins.

# Decisions
- Rust is the implementation language.
- Use Quinn QUIC over UDP with no public gateway, compute relay, permanent controller, or permanent master.
- Use one native `mesh` desktop application built with egui through eframe.
- `cargo run --release` is the source startup contract; normal onboarding requires no separate commands.
- The GUI is thin; the Tokio node runtime owns networking, hardware, model, and inference behavior.
- Enrollment uses pasted text, a `.mesh-invite` file, or a `mesh://` URI containing reachable peer details.
- Prefer single-node inference, then full-model replicas, then a continuous-layer WAN pipeline.
- Resolve provider models to immutable revisions; selected nodes download assigned tensors directly and in parallel.
- Use Safetensors first for partial downloads and propose Hugging Face Hub as the first provider.
- Use native CUDA and Metal paths; evaluate Candle first for inference.
- Distributed training remains deferred.

# Gotchas
- A short enrollment code cannot work without a public lookup service; the invitation must carry endpoint details.
- Automatic enrollment cannot cross every CGNAT or firewall; the GUI provides guided recovery without claiming success.
- Provider credentials stay local and are requested only when a selected gated model requires them.
- Layer placement increases model capacity but may increase per-token delay.
- The repository rule requires this handoff snapshot even though it is not project architecture documentation.

# Next
1. Select the first model family, model size, and correctness format.
2. Define the Model Family Adapter and layer-to-tensor mapping.
3. Define wire serialization, activation framing, tokenizer, sampling, and KV-cache contracts.
4. Confirm Hugging Face Hub and define manifest generation and partial validation.
5. Select router-mapping crates and finalize invite identity and encoding.
6. Scaffold P01 only after the next model and protocol choices are accepted.
