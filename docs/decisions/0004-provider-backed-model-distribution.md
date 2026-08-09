# ADR-0004: Immutable Provider-Backed Model Distribution

| Field | Value |
|---|---|
| Status | Accepted architecture; first adapter proposed |
| Date | 2026-08-09 |
| Owners | Architecture discussion |

## Context

A layer pipeline needs different model tensors on different PCs. Users should select a provider model once. Selected nodes should prepare their assigned weights automatically and consistently.

A moving provider reference can change between downloads. Normal provider shards may also contain tensors from several layers.

## Decision

Introduce a `ModelProvider` boundary and a normalized Mesh Model Manifest.

The coordinator resolves the user reference to an immutable provider revision before placement. Every selected node receives that exact revision, manifest hash, and its tensor assignment.

Selected nodes download in parallel. The coordinator does not relay model bytes.

Use this acquisition order:

1. Exact local cache.
2. Layer-aligned provider artifact.
3. Safetensors HTTP byte ranges.
4. Complete containing provider shard.
5. Verified peer cache when implemented and measured.

Use Hugging Face Hub as the first proposed provider adapter. Use Safetensors as the first format supporting tensor-level partial downloads.

Canonical architecture: [Provider-backed model distribution](../architecture/inference/model-distribution.md)

## Synchronization decision

Inference weights are immutable. Nodes synchronize model identity, assignment identity, and readiness. They do not repeatedly exchange unchanged weights.

The deployment commits only when every selected node reports the expected:

- Immutable revision.
- Manifest hash.
- Assignment hash.
- Loaded backend and runtime.
- Successful warm-up.

## Rejected: coordinator relays all weights

This wastes the coordinator's upload bandwidth, creates an avoidable bottleneck, and makes model preparation depend on one PC remaining online for every byte.

## Rejected: resolve `main` independently on every node

The provider may update the branch between node downloads. Nodes could load different weights while believing they loaded the same named model.

## Rejected: Model Store chooses layer placement

The Model Store lacks the complete resource and network view. The Placement Planner assigns layers. The Model Store only acquires, verifies, caches, and loads them.

## Consequences

- Provider revisions are pinned before resource commit.
- Partial download requires format-aware metadata and provider range support.
- A node may fall back to downloading a complete containing shard.
- Tied or global weights may be duplicated.
- Provider credentials remain local to each node.
- Valid disk cache survives a failed deployment preparation.
