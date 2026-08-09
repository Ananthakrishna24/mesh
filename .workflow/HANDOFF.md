# Goal
Design an extendable decentralized hardware mesh architecture for direct GPU compute across internet-connected PCs.

# Done
- Built the agent-readable `docs/` architecture knowledge base (`ec12b82`).
- Documented direct Quinn/QUIC peer connections (`ec12b82`).
- Documented distributed LLM inference and provider-backed partial model distribution (`6363c89`).
- Documented one-command native desktop onboarding and the complete roadmap (`31699c2`).
- Required native Windows and Linux NVIDIA CUDA plus macOS Apple Silicon Metal (`e3cd519`).
- Selected public dense Qwen3 with `Qwen/Qwen3-4B` for complete-model/backend proofs and `Qwen/Qwen3-8B` for distributed layer-pipeline proof (`8bbfaf1`).
- Defined the first Qwen3 profile: unquantized Safetensors, FP16 runtime weights and wire activations, 4,096-token limit, batch size 1, and non-thinking mode (`8bbfaf1`).
- Accepted Hugging Face Hub as the first provider and documented the stage-aware Qwen3 adapter contract (`8bbfaf1`).

# In progress
- Architecture discussion continues before Rust scaffolding begins.

# Decisions
- Rust is the implementation language.
- Use Quinn QUIC over UDP with no public gateway, compute relay, permanent controller, or permanent master.
- Use one native `mesh` desktop application built with egui through eframe.
- Native Windows x64 and Linux x64 NVIDIA CUDA and macOS Apple Silicon Metal are required.
- Use Candle for the first Qwen3 proof, subject to native platform validation.
- Use Qwen3-4B as the normal development and single-node proof model.
- Use Qwen3-8B as the distributed acceptance model across at least two direct peers.
- Use one mesh-owned dense Qwen3 Model Family Adapter that constructs only assigned layers.
- Resolve public Hugging Face models to immutable revisions and use Safetensors partial ranges or complete-shard fallback.
- Start Windows CI after the first confident native Windows Qwen3-4B CUDA implementation; manual Windows proofs remain required before then.
- Select one 4-bit format only after the unquantized Qwen3-8B pipeline works.
- Distributed training remains deferred.

# Gotchas
- Qwen3-4B is approximately 8 GB and Qwen3-8B approximately 16.4 GB at 16-bit parameter storage before KV cache and runtime overhead.
- Candle's complete Qwen3 model builds every layer; the mesh must implement a stage-aware loader rather than loading and discarding unassigned layers.
- Cross-backend numerical differences may change close sampling decisions; stage tensors use tolerances rather than exact token equality.
- A short enrollment code cannot work without a public lookup service; the invitation carries endpoint details.
- Automatic enrollment cannot cross every CGNAT or firewall; the GUI provides guided recovery.
- The repository rule requires this handoff snapshot even though it is not project architecture documentation.

# Next
1. Define protocol serialization and versioning.
2. Finalize Node ID, certificate, invitation encoding, and persistent-state contracts.
3. Define activation framing and Qwen3 tokenizer, sampling, and KV-cache wire contracts.
4. Define exact Hugging Face manifest generation and partial-range validation.
5. Select router-mapping crates.
6. Scaffold P01 after these immediate protocol defaults are accepted.
