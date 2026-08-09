# Goal
Design an extendable decentralized hardware mesh architecture for direct GPU compute across internet-connected PCs.

# Done
- Built the agent-readable `docs/` architecture knowledge base (`ec12b82`).
- Documented direct Quinn/QUIC peer connections (`ec12b82`).
- Documented distributed LLM inference and provider-backed partial model distribution (`6363c89`).
- Documented one-command native desktop onboarding and the complete architecture and implementation roadmap (`31699c2`).
- Made native Windows x64 NVIDIA CUDA, Linux x64 NVIDIA CUDA, and macOS Apple Silicon Metal required first-class targets (`e3cd519`).
- Added Windows-specific GUI, firewall, storage, credentials, NVML, CUDA, CI, packaging, and cross-platform proof requirements (`e3cd519`).

# In progress
- Architecture discussion continues before Rust scaffolding begins.

# Decisions
- Rust is the implementation language.
- Use Quinn QUIC over UDP with no public gateway, compute relay, permanent controller, or permanent master.
- Use one native `mesh` desktop application built with egui through eframe.
- `cargo run --release` is the source startup contract; packaged users open one application.
- Native Windows through `x86_64-pc-windows-msvc` with NVIDIA CUDA is required; WSL-only support is insufficient.
- Native Linux x64 NVIDIA CUDA and macOS Apple Silicon Metal are also required.
- Prefer single-node inference, then full-model replicas, then a continuous-layer WAN pipeline.
- Resolve provider models to immutable revisions; selected nodes download assigned tensors directly and in parallel.
- Use Safetensors first for partial downloads and propose Hugging Face Hub as the first provider.
- Candle remains proposed and must pass native Windows and Linux CUDA plus macOS Metal proofs.
- Distributed training remains deferred.

# Gotchas
- Packaged Windows users should need only a compatible NVIDIA driver; source CUDA builds require normal MSVC and CUDA development prerequisites.
- If Candle fails the native Windows proof, the Windows CUDA implementation must change behind the compute boundary rather than dropping Windows.
- A short enrollment code cannot work without a public lookup service; the invitation carries endpoint details.
- Automatic enrollment cannot cross every CGNAT or firewall; the GUI provides guided recovery.
- Layer placement increases model capacity but may increase per-token delay.
- The repository rule requires this handoff snapshot even though it is not project architecture documentation.

# Next
1. Select the first model family, model size, and correctness format across Windows CUDA, Linux CUDA, and macOS Metal.
2. Define the Model Family Adapter and layer-to-tensor mapping.
3. Define wire serialization, activation framing, tokenizer, sampling, and KV-cache contracts.
4. Confirm Hugging Face Hub and define manifest generation and partial validation.
5. Select router-mapping crates and finalize invite identity and encoding.
6. Scaffold P01 only after the next model and protocol choices are accepted.
