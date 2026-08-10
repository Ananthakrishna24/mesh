# ADR-0015: Provider Manifest, Partial Download, and Cache Policy

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-08-10 |
| Owners | Architecture discussion |
| Gates | A11, A12, A13 |

## Context

P06 must download and cache provider model artifacts before single-node or distributed inference. ADR-0004 already chose immutable provider-backed distribution, Hugging Face Hub first, and Safetensors range acquisition with complete-shard fallback. The remaining gaps blocked implementation:

- How manifests are generated, versioned, hashed, and cached (A11).
- Exact partial-download validation and retry rules (A12).
- Provider-access capability reporting and local cache/eviction policy, including interaction with P05 disk leases (A13).

Without these rules, nodes could pin different revisions, accept corrupt ranges, leak credentials into peer state, or evict weights still required by a deployment.

## Decision

Accept the detailed contracts in [Provider-backed model distribution](../architecture/inference/model-distribution.md) for gates A11–A13.

### A11 — Manifest generation

- Hugging Face Hub is the only v1 provider; client work stays behind `mesh-model`.
- User references resolve once to a full 40-character commit SHA before placement or weight download.
- Safetensors headers are discovered with bounded Range reads (or complete-shard parse on fallback).
- `qwen3-dense` adapter mapping runs at an explicit `adapter_version`.
- `manifest_hash` is SHA-256 over canonical manifest bytes excluding the hash and local paths.
- Manifest cache keys include provider, repository, revision, adapter id/version, format, and quantization.

### A12 — Partial download validation

- Accept range bodies only from validated `206` + `Content-Range` responses.
- Validate length, dtype, shape, optional ETag, and digests before publish.
- Incomplete objects use `.partial` + sidecar meta and are never worker-visible.
- Retries use bounded exponential backoff inside the reservation lease; auth errors do not retry as transient.
- Complete-shard fallback triggers on unsupported/invalid ranges or when merged ranges cover most of a shard.

### A13 — Access and cache

- Credential storage remains native keyring via `mesh-store` (ADR-0010).
- Nodes report local `ProviderAccessReport`; peers learn only a non-secret readiness hint.
- Prepare disk usage goes through Local Resource Manager leases; cache hits reduce requested bytes.
- Default cache cap is unlimited soft cap with a volume reserve floor; user may set `cache_max_bytes`.
- Evict only unreferenced entries; protect active deployment and in-flight download artifacts.
- Startup and aborted prepares clean stale `.partial` files after grace or known abort.

## Rejected: resolve revision independently per node

Re-resolving `main` on each peer can split a deployment across two commits. The coordinator pins one SHA in the plan; workers only fetch that SHA.

## Rejected: trust length alone without header reparse

A range of the right length can still be the wrong tensor if offsets drift. Validation always rechecks Safetensors header dtype and shape for the tensor name.

## Rejected: coordinator-held provider token for workers

Shipping the coordinator token to stage hosts couples trust domains and expands blast radius. Each node keeps its own credential and rejects prepare when access is missing.

## Rejected: LRU eviction without reference protection

Time-only LRU can delete weights mid-prepare or mid-serve. Reference counts and deployment locks gate eviction; LRU applies only to unprotected entries.

## Rejected: multiple first providers or formats

v1 implements one Hub adapter and Safetensors only. Extra providers or GGUF paths wait until the Qwen3 proofs exist.

## Consequences

- P06 implements `mesh-model` against this contract and stores manifest/cache rows in `mesh-store`.
- Capability snapshots may gain a boolean Hugging Face readiness hint without protocol-breaking credential fields.
- Disk reservation amounts in prepare plans must net out verified local cache hits.
- Adapter upgrades create new manifest hashes; active deployments remain pinned to the old hash.
- Peer cache sharing stays deferred; provider download remains the first source.
- Checklist gates A11–A13 can be marked resolved; P06 may start.
