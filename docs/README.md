# Hardware Mesh Documentation

| Field | Value |
|---|---|
| Status | Active design |
| Current phase | P07 single-node inference in progress; Linux path coded |
| Implementation language | Rust |
| First GPU targets | NVIDIA CUDA on Windows and Linux; Apple Metal on macOS |


This folder is the project memory. Read it before changing architecture or code.

## Agent reading order

1. [Architecture overview](architecture/README.md)
2. [Node modules](architecture/system/node-modules.md)
3. [Persistent state](architecture/system/persistent-state.md)
4. [Direct connection algorithm](architecture/networking/direct-connection.md)
5. [NAT and router mapping](architecture/networking/nat-router-mapping.md)
6. [Peer-record merge rules](architecture/networking/peer-record-merge.md)
7. [Network benchmark and placement cost](architecture/networking/network-benchmark.md)
8. [Control protocol](architecture/protocol/control-protocol.md)
9. [Desktop onboarding](architecture/onboarding/README.md)
10. [Enrollment contract](architecture/onboarding/enrollment-contract.md)
11. [Distributed LLM inference](architecture/inference/README.md)
12. [Activation tensor frame](architecture/protocol/activation-frame.md)
13. [Qwen3 model family](architecture/inference/qwen3-model-family.md)
14. [Tokenizer and sampling ownership](architecture/inference/tokenizer-and-sampling.md)
15. [KV-cache contract](architecture/inference/kv-cache.md)
16. [Provider-backed model distribution](architecture/inference/model-distribution.md)
17. [Inference parallelism and edge cases](architecture/inference/parallelism-and-edge-cases.md)
18. [GPU backends](architecture/compute/gpu-backends.md)
19. [Architecture and implementation roadmap](implementation/roadmap.md)
20. [Roadmap implementation checklist](implementation/checklist.md)
21. [Rust workspace plan](implementation/rust-workspace.md)
22. [Architecture decisions](decisions/)

## Source-of-truth map

| Question | Canonical document |
|---|---|
| What are we building now? | [Architecture overview](architecture/README.md) |
| What runs inside each PC? | [Node modules](architecture/system/node-modules.md) |
| Where is durable state stored? | [Persistent state](architecture/system/persistent-state.md) |
| How do two PCs connect? | [Direct connection algorithm](architecture/networking/direct-connection.md) |
| How is automatic router mapping done? | [NAT and router mapping](architecture/networking/nat-router-mapping.md) |
| How are peer records and candidates merged? | [Peer-record merge rules](architecture/networking/peer-record-merge.md) |
| How are links measured for placement? | [Network benchmark and placement cost](architecture/networking/network-benchmark.md) |
| How are control messages framed and versioned? | [Control protocol](architecture/protocol/control-protocol.md) |
| How does a user start and enroll a PC? | [Desktop onboarding](architecture/onboarding/README.md) |
| What exactly is shared during enrollment? | [Enrollment contract](architecture/onboarding/enrollment-contract.md) |
| How is an LLM placed and run? | [Distributed LLM inference](architecture/inference/README.md) |
| What is the activation tensor wire format? | [Activation tensor frame](architecture/protocol/activation-frame.md) |
| Which models are the first proofs? | [Qwen3 model family](architecture/inference/qwen3-model-family.md) |
| Who tokenizes and samples? | [Tokenizer and sampling ownership](architecture/inference/tokenizer-and-sampling.md) and [ADR-0016](decisions/0016-tokenizer-sampling-kv-cache.md) |
| How is KV cache laid out and sized? | [KV-cache contract](architecture/inference/kv-cache.md) and [ADR-0016](decisions/0016-tokenizer-sampling-kv-cache.md) |
| How are partial model weights downloaded and synchronized? | [Provider-backed model distribution](architecture/inference/model-distribution.md) |
| Which inference work can run in parallel? | [Inference parallelism and edge cases](architecture/inference/parallelism-and-edge-cases.md) |
| Why QUIC and Quinn? | [ADR-0001](decisions/0001-direct-quic-transport.md) |
| How do NVIDIA and Metal fit? | [GPU backends](architecture/compute/gpu-backends.md) |
| Is Windows NVIDIA a required target? | [ADR-0006](decisions/0006-windows-nvidia-required.md) |
| How should the Rust repository be split? | [Rust workspace plan](implementation/rust-workspace.md) |
| What remains to be decided and built? | [Architecture and implementation roadmap](implementation/roadmap.md) |
| How is roadmap implementation progress tracked? | [Roadmap implementation checklist](implementation/checklist.md) |
| How are provider manifests, range downloads, and cache eviction decided? | [Provider-backed model distribution](architecture/inference/model-distribution.md) and [ADR-0015](decisions/0015-provider-manifest-download-cache.md) |


Do not copy a rule into several documents. Update the canonical document and link to it.

## Fixed constraints

- PCs may be in different physical locations.
- Compute data moves directly between PCs.
- There is no required public gateway, relay, coordinator, or permanent master.
- Every PC runs the same node software.
- A job creator may control its own job. It does not control the mesh.
- Rust is the implementation language.
- NVIDIA CUDA on Windows and Linux and Apple Metal on macOS are required first-class GPU targets.
- LLM inference architecture is accepted but not implemented. Distributed training remains deferred.
- The primary product is one native desktop application with guided onboarding; separate commands are not required for normal use.
- Qwen3-4B is the first complete-model proof; Qwen3-8B is the first distributed layer-pipeline proof.

## Agent rules

Before implementing a change:

1. Read the canonical document for the area.
2. Check the decision records for rejected approaches.
3. Preserve every fixed constraint.
4. Mark a new choice as `Proposed` until it is accepted.
5. Update documentation in the same change as the code.
6. Do not silently add a central service or relay.
7. Do not claim that remote GPUs behave like one local GPU.

## Status words

- **Accepted:** use this choice unless a new decision replaces it.
- **Proposed:** preferred direction, but not yet locked.
- **Deferred:** intentionally outside the current phase.
- **Rejected:** considered and not selected.

## Small glossary

- **Peer:** one PC running the mesh node.
- **Node:** the software running on a peer.
- **Candidate address:** an address another PC can try.
- **Direct link:** one PC-to-PC connection with no data relay.
- **Known peer:** a peer stored in the local peer list.
- **Job owner:** the PC that created one specific job.
- **Hole punching:** both PCs send packets so their routers may allow a direct path.
- **Model stage:** one continuous range of layers from the same model.
- **Placement plan:** nodes, layer ranges, route, memory, and immutable model identity for one deployment.
- **Model provider:** an external source of model metadata and artifacts.
- **Ready barrier:** inference starts only after every required stage reports the expected model and ready state.
