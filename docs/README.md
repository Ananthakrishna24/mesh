# Hardware Mesh Documentation

| Field | Value |
|---|---|
| Status | Active design |
| Current phase | Hardware mesh and direct peer connections |
| Implementation language | Rust |
| First GPU targets | NVIDIA CUDA and Apple Metal |

This folder is the project memory. Read it before changing architecture or code.

## Agent reading order

1. [Architecture overview](architecture/README.md)
2. [Node modules](architecture/system/node-modules.md)
3. [Direct connection algorithm](architecture/networking/direct-connection.md)
4. [GPU backends](architecture/compute/gpu-backends.md)
5. [Rust workspace plan](implementation/rust-workspace.md)
6. [Architecture decisions](decisions/)

## Source-of-truth map

| Question | Canonical document |
|---|---|
| What are we building now? | [Architecture overview](architecture/README.md) |
| What runs inside each PC? | [Node modules](architecture/system/node-modules.md) |
| How do two PCs connect? | [Direct connection algorithm](architecture/networking/direct-connection.md) |
| Why QUIC and Quinn? | [ADR-0001](decisions/0001-direct-quic-transport.md) |
| How do NVIDIA and Metal fit? | [GPU backends](architecture/compute/gpu-backends.md) |
| How should the Rust repository be split? | [Rust workspace plan](implementation/rust-workspace.md) |

Do not copy a rule into several documents. Update the canonical document and link to it.

## Fixed constraints

- PCs may be in different physical locations.
- Compute data moves directly between PCs.
- There is no required public gateway, relay, coordinator, or permanent master.
- Every PC runs the same node software.
- A job creator may control its own job. It does not control the mesh.
- Rust is the implementation language.
- NVIDIA CUDA and Apple Metal are the first GPU targets.
- LLM inference and distributed training are later phases.

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
