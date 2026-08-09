# Goal
Design an extendable decentralized hardware mesh architecture for direct GPU compute across internet-connected PCs.

# Done
- Replaced the abandoned HTML page with the agent-readable `docs/` knowledge base (`ec12b82`).
- Documented the direct Quinn/QUIC peer connection algorithm (`ec12b82`).
- Documented distributed LLM inference modes, placement, local reservations, batching, pipeline execution, and failure rules (`6363c89`).
- Documented provider-backed partial model distribution, immutable revision synchronization, Safetensors range loading, caching, and readiness barriers (`6363c89`).
- Added Rust crate boundaries for model providers and inference coordination (`6363c89`).

# In progress
- Architecture discussion continues before Rust scaffolding begins.

# Decisions
- Rust is the implementation language.
- Use Quinn QUIC over UDP for direct peer transport.
- Do not use a public gateway, compute relay, permanent controller, or permanent master.
- Prefer single-node inference, then full-model replicas, then a continuous-layer WAN pipeline.
- Each peer owns local expiring resource reservations.
- Resolve provider models to immutable revisions before placement.
- Selected nodes download assigned model tensors directly and in parallel.
- Use Safetensors as the first partial-download format; propose Hugging Face Hub as the first provider adapter.
- Synchronize model identity, assignment, and readiness; inference weights remain immutable.
- Use native CUDA and Metal paths; evaluate Candle first for inference.
- Distributed training remains deferred.

# Gotchas
- Layer placement increases model capacity but may increase per-token delay.
- A pipeline stage failure loses that stage's KV cache; the first version restarts affected requests.
- Provider shards may not align with layers; use Safetensors ranges or download the complete containing shard.
- At least one initial peer must be directly reachable; no relay fallback exists.
- The repository rule requires this handoff snapshot even though it is not project architecture documentation.

# Next
1. Define control-message and inference-message wire formats with versioning.
2. Select the first supported model family and quantization for an end-to-end prototype.
3. Confirm Hugging Face Hub as the first provider and define the exact manifest-generation path.
4. Select router-mapping crates for UPnP, NAT-PMP, and PCP.
5. Scaffold the Rust workspace after the remaining protocol choices are accepted.
