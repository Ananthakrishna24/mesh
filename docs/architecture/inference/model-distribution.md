# Provider-Backed Model Distribution

| Field | Value |
|---|---|
| Status | Accepted |
| Canonical for | Resolving, partitioning, downloading, caching, and synchronizing model weights |
| Parent | [Distributed LLM inference](README.md) |
| Decision | [ADR-0004: immutable provider-backed model distribution](../../decisions/0004-provider-backed-model-distribution.md) |
| Gate decision | [ADR-0015: provider manifest, partial download, and cache policy](../../decisions/0015-provider-manifest-download-cache.md) |
| Implements gates | A11, A12, A13 |

## Goal

A user selects a model from a provider. The mesh resolves one exact version, selects inference nodes, and makes each selected node download and load only the tensors assigned to it when the model format and provider allow partial access.

This allows a layer pipeline to run a model larger than one node's usable GPU memory.

## Architecture

```text
User model reference
        │
        ▼
Inference Coordinator
├── Model Provider Adapter ───────▶ Model provider
├── Model Resolver                 metadata and artifacts
├── Placement Planner
└── Deployment Manifest
        │
        ├──────── PREPARE stage A ───────▶ PC A Model Store
        ├──────── PREPARE stage B ───────▶ PC B Model Store
        └──────── PREPARE stage C ───────▶ PC C Model Store
                                                   │
                  Each selected PC downloads directly from the provider
                  or reads verified tensors from its local or peer cache.
```

The coordinator does not download and relay every model byte. Selected nodes prepare in parallel.

## Module responsibilities

### Model Provider Adapter

Provides one internal interface over an external model provider.

```rust
trait ModelProvider {
    async fn resolve(&self, reference: &ModelReference) -> Result<ResolvedModel>;
    async fn read_metadata(&self, artifact: &ArtifactRef) -> Result<ArtifactMetadata>;
    async fn fetch_file(&self, artifact: &ArtifactRef, destination: &Path) -> Result<()>;
    async fn fetch_range(&self, artifact: &ArtifactRef, range: Range<u64>) -> Result<Bytes>;
    async fn probe_access(&self, reference: &ModelReference) -> Result<ProviderAccessState>;
}
```

The exact Rust trait may change during implementation. The behavior is accepted.

A provider adapter must support:

- Resolving a branch or tag to an immutable revision.
- Listing required files.
- Downloading selected files.
- Optional byte-range reads.
- Provider authentication without sending credentials to other peers.
- Retry and resumable download where the provider supports it.
- Local access probing for capability reporting.

### Model Resolver

Creates one normalized Mesh Model Manifest. It understands model structure, not node capacity.

### Placement Planner

Assigns layers and shared tensors to nodes. It creates the per-node artifact plan.

### Model Store

Executes the artifact plan, verifies artifacts, caches them, loads tensors, and reports readiness. It never changes layer placement.

## Immutable model identity

A model used by a deployment is identified by:

```rust
struct ModelIdentity {
    provider: String,           // "huggingface"
    repository: String,         // "Qwen/Qwen3-8B"
    revision: String,           // full 40-char commit SHA
    manifest_hash: String,      // lowercase hex SHA-256 of canonical manifest bytes
    model_format: ModelFormat,  // Safetensors for the first path
    quantization: Option<String>,
    tokenizer_hash: String,     // lowercase hex SHA-256 over pinned tokenizer artifact bytes
}
```

Never run a deployment from a moving name such as `main` alone. Resolve it once, pin the returned immutable revision, and send that revision to every node.

If the provider changes `main`, active deployments continue using their pinned revision. A new revision creates a new deployment.

## Mesh Model Manifest

The manifest normalizes provider files into model parts.

It contains:

- Immutable model identity fields except `manifest_hash` itself.
- Architecture and configuration snapshot used by the adapter.
- Number of layers and hidden size.
- Supported data types and quantization.
- Tokenizer artifact references and `tokenizer_hash` inputs.
- Global tensors such as embeddings, final normalization, and output head.
- Tensor name, shape, data type, layer ownership, source artifact, and byte range.
- Whole-artifact size, ETag, and digest when available.
- Per-tensor or range digest when available.
- Loaded-memory estimate.
- Runtime and operation requirements.
- `adapter_id` and `adapter_version` that produced the mapping.

The manifest hash is part of every placement and readiness message.

## A11 — Provider manifest generation

### First provider

Hugging Face Hub is the first and only v1 provider adapter.

| Field | Value |
|---|---|
| Provider id | `huggingface` |
| Client | Rust `hf-hub` for repo metadata, complete-file download, and local HF cache integration |
| Auth | Optional read token from the native credential store key `mesh.model-provider.huggingface` / `default` |
| First models | `Qwen/Qwen3-4B`, `Qwen/Qwen3-8B` |
| First format | Upstream Safetensors, including sharded `model.safetensors.index.json` layouts |

Provider-specific types (`hf_hub::*`, raw Hub JSON) must not leave `mesh-model`. Planning, store, and wire types use mesh-owned records only.

### Immutable revision resolution

1. Accept a user reference `{ provider = "huggingface", repository, revision_hint }`.
2. Default `revision_hint` is `main` when the user does not pin one.
3. Call Hub repo info for that repository and revision hint.
4. Require a full 40-character lowercase hexadecimal Git commit SHA. Reject abbreviated SHAs and symbolic names as final identity.
5. Pin `ModelIdentity.revision` to that SHA before any weight download or placement offer.
6. All later artifact URLs, cache keys, and peer prepare messages use the pinned SHA, never the original branch or tag.
7. If repo info fails with authentication or gate errors, surface `ProviderAccessState` and stop resolution. Do not invent a revision.

A node that receives a prepare plan with a non-SHA revision rejects preparation.

### Required provider files

For the first Qwen3 path the resolver must obtain, at the pinned revision:

| Artifact | Role |
|---|---|
| `config.json` | Architecture fields for the Model Family Adapter |
| `tokenizer.json` plus tokenizer sidecars referenced by it | Tokenizer identity |
| `model.safetensors` **or** `model.safetensors.index.json` and every listed shard | Weights |
| Optional chat/template files required by the Qwen3 non-thinking profile | Prompt formatting |

Missing required files fail resolution with an actionable error. Extra repo files are ignored.

### Safetensors header discovery

For each weight artifact at the pinned revision:

1. If `model.safetensors.index.json` exists, download the complete index and build the tensor → shard map from `weight_map`.
2. For every needed shard or single-file weights blob:
   - `GET` bytes `[0, 8)` and decode a little-endian `u64` header length `H`.
   - Reject `H == 0` or `H > 100_000_000`.
   - `GET` bytes `[8, 8 + H)` and parse JSON as the Safetensors header.
   - For each tensor entry read `dtype`, `shape`, and `data_offsets = [start, end)` relative to the start of the tensor payload region.
   - Absolute file offsets are:
     - payload base = `8 + H`
     - absolute start = `payload_base + start`
     - absolute end = `payload_base + end`
   - Byte length = `absolute_end - absolute_start`.
3. Cross-check sharded index membership: every `weight_map` tensor must appear in exactly one shard header; every required adapter tensor must appear in the index or single-file header.
4. Store per-artifact `content_length` when the provider returns it, plus `ETag` when present, on the manifest artifact records.

Header discovery may use HTTP Range. If Range is unsupported for a shard, download that complete shard once, parse the header from disk, and keep the shard as a cache candidate under complete-shard rules.

### Model Family Adapter mapping

After headers and `config.json` are available:

1. Select the adapter with `adapter_id = "qwen3-dense"` when `config.model_type` identifies Qwen3 dense. Unknown families fail closed.
2. Run the adapter at a concrete `adapter_version` (semver string embedded in `mesh-model`).
3. Map every required logical tensor to layer ownership, global role, dtype, shape, source artifact, and absolute byte range.
4. Apply Qwen3 ownership rules from [Qwen3 dense model family](qwen3-model-family.md): first-stage embeddings, continuous decoder layers, final RMSNorm, `lm_head` or tied embeddings.
5. Compute loaded-memory estimates from tensor byte lengths plus adapter overhead rules.
6. Fail if required tensors are missing, duplicated across conflicting ranges, or disagree with `config.json` layer counts and hidden size.

The adapter does not download weights beyond headers and config needed for mapping. Weight bytes are acquired later by the Model Store from the per-node plan.

### Canonical manifest bytes and hash

`manifest_hash = SHA-256(canonical_manifest_bytes)` encoded as lowercase hex.

Canonical bytes are the deterministic CBOR (preferred) or canonical JSON encoding of the Mesh Model Manifest **with these fields omitted from the hashed body**:

- `manifest_hash` itself
- local filesystem paths
- node-specific cache state
- wall-clock generation timestamps

Hashed body must include:

- `provider`, `repository`, `revision`
- `adapter_id`, `adapter_version`
- `model_format`, `quantization`
- architecture snapshot fields the adapter used
- ordered tensor records: name, dtype, shape, layer or global role, artifact id, absolute range, optional range digest
- ordered artifact records: relative path, size, optional ETag, optional whole-file digest
- tokenizer artifact set and `tokenizer_hash`
- memory estimate summary

Tensor records are sorted by tensor name ascending. Artifact records are sorted by relative path ascending. Equivalent manifests produced on different nodes must hash identically.

`ModelIdentity.manifest_hash` is that digest. Placement, prepare, and ready messages carry the full identity including this hash.

### Manifest cache key

Persist and reuse a generated manifest under:

```text
manifest_cache_key =
  provider || ":" || repository || ":" || revision
  || ":adapter=" || adapter_id || "@" || adapter_version
  || ":fmt=" || model_format
  || ":quant=" || quantization_or_none
```

Rules:

- Cache hit requires the key match **and** stored `manifest_hash` re-hashes to the same value from stored canonical bytes.
- Different `adapter_version` values never share a manifest entry.
- Provider file edits at the same commit SHA are treated as corruption or provider inconsistency: hash mismatch fails the entry and forces regeneration; repeated mismatch fails resolution.
- Manifest rows live in SQLite (`model_manifests`). Canonical bytes may live in SQLite or as a file under the cache root referenced by the row.

### Adapter-version behavior

| Event | Behavior |
|---|---|
| Same adapter version | Reuse valid cached manifest for the key |
| Mesh upgrades `adapter_version` | Old manifests remain readable for active deployments pinned to them; new resolutions use the new version and new hash |
| Active deployment pinned to old hash | Continue; do not silently remap tensors under a new adapter |
| Adapter cannot read an old pinned manifest | Fail preparation truthfully; operator must recreate the deployment |
| Removing an adapter id | Fail resolution for new references; existing pinned deployments still need their cached manifest bytes |

v1 ships only `qwen3-dense`. Additional families require a new adapter id, not a silent reuse of the Qwen3 mapper.

## A12 — Partial download validation

### Acquisition order

Unchanged from ADR-0004:

1. Exact local cache hit for the assigned tensor set or containing artifact.
2. Layer-aligned complete provider artifact when the assignment fills it.
3. Safetensors HTTP byte ranges for assigned tensors.
4. Complete containing provider shard fallback.
5. Verified peer cache when later enabled and measured (deferred).

### Range request rules

- Issue HTTP `Range: bytes=start-end_inclusive` where `end_inclusive = absolute_end - 1`.
- Accept only `206 Partial Content` for range attempts.
- Require `Content-Range: bytes start-end/total` and verify:
  - `start` and `end` match the request
  - `end - start + 1` equals the received body length
  - `total` equals the artifact `content_length` when both are known
- If the server answers `200 OK` with a full body to a range request, treat Range as unsupported for that artifact and enter complete-shard fallback. Do not silently accept a partial prefix of a `200` body as a range.
- If the server answers `416` or an unparsable `Content-Range`, fail the range attempt and fall back or retry per policy below.
- Maximum concurrent range requests per artifact: 4.
- Merge overlapping or adjacent required tensor ranges when the gap between them is ≤ 64 KiB before issuing HTTP calls.

### Length validation

| Object | Rule |
|---|---|
| Complete file | Final byte length must equal provider `content_length` when known; otherwise equal the sum of header plus payload implied by the Safetensors header |
| Range body | Body length must equal requested byte count |
| Tensor payload | `absolute_end - absolute_start` must equal the product of shape dimensions times dtype width, except when the adapter explicitly allows padded provider layouts (v1 allows none) |
| Cache entry | Stored `byte_length` must match the on-disk file length before the entry is `VALID` |

Mismatch deletes the temporary file and fails or retries the attempt. A committed `VALID` row whose file length later mismatches becomes `INVALID`.

### Shape and data-type validation

Before a tensor range is marked valid:

1. Manifest dtype and shape are the source of truth from header discovery.
2. Re-read the local Safetensors header (from range cache or complete shard) and require identical dtype and shape for that tensor name.
3. Dtype widths used by validation:

| dtype | bytes |
|---|---:|
| BOOL, U8, I8 | 1 |
| I16, U16, F16, BF16 | 2 |
| I32, U32, F32 | 4 |
| I64, U64, F64 | 8 |

4. First correctness profile expects provider weight dtypes compatible with FP16 runtime conversion (BF16 or F16 preferred). Unsupported dtypes fail preparation rather than guessing.

### ETag handling

- Store provider `ETag` on artifact metadata when present (including weak etags, preserved verbatim).
- On cache reuse of a complete artifact, revalidate when the entry is older than 24 hours **or** when preparation begins for a new deployment: issue a conditional request (`If-None-Match`) or re-fetch metadata. `304` keeps the entry; changed ETag invalidates the complete-file entry and all ranges derived only from that ETag.
- Range objects inherit the parent artifact ETag observed at download time. Parent ETag change invalidates child ranges.
- Missing ETag is allowed. Then immutability rests on pinned revision + digest/length checks only.

### Digest validation

Preference order for integrity:

1. Provider whole-file SHA-256 or SHA-1 when Hub metadata supplies it for the exact revision path.
2. Manifest-recorded whole-file digest from resolution time.
3. Optional per-tensor digest when recorded in the manifest (v1 may omit if provider does not supply one).

Rules:

- When a digest is present, compute it over the final file or extracted tensor bytes before atomic publish.
- Digest mismatch deletes the temporary object, marks any existing entry `INVALID`, and fails the attempt.
- When no digest exists, length + header reparse + pinned revision are required; preparation still proceeds for public first models.
- `tokenizer_hash` is always computed locally over the exact tokenizer artifact bytes used.

### Incomplete-file handling

- In-progress downloads use a sibling name ending in `.partial` under the cache root.
- Partial files are never exposed to the Inference Worker.
- A partial file may be resumed only when a sidecar `.partial.meta` records artifact identity, revision, absolute range or full-file flag, expected length, ETag, and next offset. Meta mismatch truncates and restarts.
- Process crash leaves partials on disk; startup recovery applies incomplete-download cleanup (A13).
- SQLite must not contain `VALID` rows pointing at `.partial` paths.

### Retry behavior

| Failure class | Policy |
|---|---|
| Transient network / 5xx / timeout | Exponential backoff: 500 ms, 2 s, 8 s; max 3 attempts per artifact action inside the reservation lease |
| `429` rate limit | Honor `Retry-After` when present, otherwise 5 s, 20 s; max 3 attempts |
| `401` / `403` | No retry loop; set provider access state and fail preparation |
| `404` at pinned revision | Fail immediately; revision/artifact inconsistency |
| Range unsupported or invalid response | One clean fallthrough to complete-shard path, not repeated range retries |
| Digest / length / header mismatch | Delete temp; at most one full restart of that artifact action; second failure fails preparation |
| Disk full | Fail immediately; do not evict active artifacts to make space during the same preparation |

All retries must finish or fail before the resource reservation lease expires. Lease expiry aborts downloads and runs incomplete cleanup for that deployment's temps.

### Complete-shard fallback

Trigger when any of:

- Provider rejects Range
- Range responses fail validation
- Merged ranges cover ≥ 80% of the shard size (download the whole shard instead)

Behavior:

1. Download the complete shard to a `.partial` path with resume support.
2. Validate length, optional digest, ETag, and full Safetensors header.
3. Publish the shard as a `VALID` complete artifact cache entry.
4. Satisfy assigned tensors from that shard without retaining separate range objects for those tensors.
5. Keep the shard for reuse; do not delete unassigned tensors from the shard on disk.
6. Runtime loading still maps only assigned tensors into the stage.

## A13 — Provider access and local cache

Credential persistence remains canonical in [Persistent state](../system/persistent-state.md). This section locks capability reporting, disk reservation interaction, cache limits, eviction, active protection, and incomplete cleanup.

### Provider-access capability reporting

Each node maintains local-only provider access state. v1 reports Hugging Face only.

```text
ProviderAccessReport
├── provider: "huggingface"
├── checked_at_unix_ms
├── auth_mode: none | session | saved
├── public_read: bool
├── gated_read: bool
├── status: ready | needs_token | invalid_token | store_unavailable | unchecked
└── detail: short user-facing string
```

Rules:

- Public Qwen3 models require `public_read = true` after a successful unauthenticated metadata probe, or after any successful authenticated probe.
- Gated or private refs require a validated token and `gated_read = true` for that node.
- Capability exchange with peers may include a boolean summary `provider_huggingface_ready` on the local capability snapshot for planner hints.
- Peers never receive tokens, token fingerprints, or raw error bodies that might leak credentials.
- A node with `status != ready` for the required access class must reject `MODEL_PREPARE` before download.
- Refresh access state on startup, after credential save/delete, and before the first prepare that needs the provider.
- Planner preference: do not select a peer that reports not-ready when another capable peer exists. The authoritative reject remains the preparing node.

### Disk reservation

Model preparation consumes disk through the Local Resource Manager:

1. The Placement Planner sums `disk_bytes_required` per node from assigned complete shards or merged ranges, plus a 64 MiB or 1% whichever larger safety margin.
2. Already-`VALID` local cache hits that cover an assignment reduce the reservation request for those bytes.
3. `RESOURCE_QUERY` / `RESERVE_REQUEST` include the net additional disk bytes.
4. The Model Store must not start a download that would exceed the reserved disk amount for that deployment.
5. On prepare failure or release, disk reservations clear with the rest of the lease. Valid published cache entries remain on disk without holding the deployment lease.
6. Cache eviction and user cache-clear actions operate on unreserved, unreferenced entries only.

Disk available for offers still comes from `CapabilityReport.disk_available_bytes` minus active reservations, as in P05.

### Cache root and entry identity

- Cache root defaults to a sibling of `mesh.db` named `model-cache` under the application data directory. User preference may relocate it.
- Artifact files use content-addressed relative paths:

```text
objects/<provider>/<repository_hash>/<revision>/<artifact_path_hash>
ranges/<provider>/<repository_hash>/<revision>/<artifact_path_hash>/<start>_<end>
manifests/<manifest_hash>
```

- SQLite `model_cache_entries` store relative path, byte length, digests, ETag, validation state, last_used_at, and reference counts.
- Logical cache key for a complete artifact:

```text
provider + repository + revision + artifact_relative_path + optional_whole_digest
```

- Logical cache key for a tensor range adds absolute `start`, `end`, dtype, and shape.

### Cache limit

| Parameter | Default | Notes |
|---|---|---|
| `cache_max_bytes` | `0` (unlimited) | User-configurable; `0` means no soft cap beyond disk capacity |
| Effective hard ceiling | `disk_available` at eviction time | Never fill the volume below the reserve floor |
| Volume reserve floor | `max(5 GiB, 5% of disk_total)` | Free space that eviction tries to preserve |

When `cache_max_bytes > 0`, the sum of `VALID` entry lengths should stay ≤ that cap after background eviction. Unlimited mode still respects the volume reserve floor.

### Eviction thresholds

Eviction runs:

- Before preparation when projected usage would exceed `cache_max_bytes` or breach the volume reserve floor.
- On startup after incomplete cleanup.
- When the user triggers **Free cache space**.

Candidate selection:

1. Only `VALID` or `INVALID` entries with reference count 0 and no active deployment or download lock.
2. Prefer `INVALID` first.
3. Then complete shards that are older by `last_used_at` when equivalent range tensors exist for recent assignments.
4. Then oldest `last_used_at` unreferenced entries.
5. Never evict an entry whose bytes are required by an in-flight prepare that already reserved them.

Eviction deletes the file first, then the database row in one storage transaction. Orphan files without rows are deleted on startup scrub.

### Active-artifact protection

An entry is protected when any of:

- Reference count > 0 from a committed or preparing deployment
- Held by an in-flight download worker
- Marked pinned by explicit user action (optional v1 GUI later; schema allows it)
- Required by a restored durable deployment still in recovery

Protected entries are invisible to eviction. Preparation that needs space and cannot evict enough unreferenced data fails with an actionable disk error.

### Incomplete-download cleanup

On every node startup, and when a deployment prepare aborts:

1. Enumerate `*.partial` and `*.partial.meta` under the cache root.
2. Delete partials with no matching in-memory download worker after a grace period of 30 minutes since last meta mtime.
3. Delete partials immediately when their deployment id is known aborted or the reservation is expired.
4. Delete meta without partial and partial without meta.
5. Never promote a partial to `VALID` during cleanup.
6. Do not delete non-partial `VALID` artifacts during incomplete cleanup.

### User-visible cache and access actions

The GUI, through typed commands only, must support:

- Show provider access status and save/delete Hugging Face token
- Show cache root, used bytes, limit, and protected bytes
- Cancel in-flight prepare downloads for deployments this node owns or participates in as stage host
- Clear unreferenced cache entries

Leaving a mesh does not delete verified cache artifacts.

## Partial weight strategies

Use the first strategy that produces verified assigned tensors efficiently.

### 1. Local tensor cache

If the exact model revision and assigned tensor set already exist locally, reuse them.

### 2. Layer-aligned provider artifact

Download complete files when the provider already stores the assigned layers in their own artifact. This is the simplest partial distribution.

### 3. Safetensors byte ranges

Follow A11 header discovery and A12 range validation. Store verified ranges under the range cache key.

### 4. Complete provider shard fallback

Follow A12 complete-shard fallback. Keep only assigned tensors mapped at runtime; preserve the valid shard on disk.

### 5. Verified peer cache

A connected peer may offer an exact artifact or tensor range it already has. The receiver checks immutable identity, length, and available digest before accepting it.

Peer cache sharing is deferred until measured. The provider remains the first source.

## Placement and preparation flow

```text
1. RESOLVE
   User reference → immutable revision → Mesh Model Manifest

2. PLAN
   Manifest + resources + network graph → Placement Plan

3. RESERVE
   Every selected node accepts an expiring resource lease

4. PREPARE IN PARALLEL
   PC A downloads layers 0–11
   PC B downloads layers 12–27
   PC C downloads layers 28–39

5. VERIFY
   Every node verifies its assigned artifacts and manifest identity

6. LOAD IN PARALLEL
   Every node loads its assigned stage into CUDA or Metal

7. WARM UP
   Every stage runs its local warm-up

8. READY BARRIER
   The coordinator waits until every stage reports READY

9. COMMIT
   The deployment begins serving requests
```

A **ready barrier** means inference does not start until every required stage is ready.

## Per-node preparation plan

```rust
struct NodeModelPlan {
    deployment_id: DeploymentId,
    model: ModelIdentity,
    assignment_hash: String,
    first_layer: u32,
    last_layer_exclusive: u32,
    tensor_assignments: Vec<TensorAssignment>,
    global_tensors: Vec<TensorAssignment>,
    disk_bytes_required: u64,
    gpu_bytes_reserved: u64,
}
```

The first stage receives embeddings. The final stage receives final normalization and output-head tensors. Tied embeddings may need duplication; the plan accounts for that memory.

`assignment_hash` is lowercase hex SHA-256 over the canonical encoding of the deployment id, model identity, ordered layer range, and ordered tensor assignment list for that node.

## Synchronization rule

Inference weights are immutable. Nodes do not continuously synchronize weight changes.

They synchronize **identity and readiness**:

```text
MODEL_PREPARE
- deployment ID
- immutable model revision
- manifest hash
- assignment hash

MODEL_READY
- deployment ID
- immutable model revision
- manifest hash
- assignment hash
- loaded byte count
- runtime backend and version
- warm-up result
```

The coordinator commits only if every selected node reports the expected values.

The following remain local and are not globally synchronized:

- Full model files not assigned to the node.
- Local cache layout.
- Stage KV cache.
- Temporary download files.
- Provider credentials and access detail strings.

## Cache behavior summary

Cache keys include immutable revision, artifact identity, byte range or tensor identity, data type, and quantization.

Rules:

- Write downloads to an incomplete temporary path.
- Rename or mark ready only after validation.
- Never serve incomplete data to an Inference Worker.
- Reuse valid data across deployments.
- Track last use and current deployment references.
- Evict only unreferenced entries under A13 thresholds.
- Prefer evicting old full shards when verified layer tensors remain.
- Rate-limit background cache work during active inference.

## Provider credentials

Provider credentials remain local to each node. The coordinator sends model identity and artifact references, not another node's access token.

For gated models, each selected node must already have valid provider access. A node without access rejects preparation before commit. The first version does not distribute credentials between peers.

Canonical storage contract: [Persistent state](../system/persistent-state.md)

## Failure handling

| Failure | Required behavior |
|---|---|
| Provider unavailable | Retry within the reservation lease, try a verified peer source when enabled, or reject preparation |
| Node leaves during download | Release the pending plan and create a new placement |
| Disk space exhausted | Reject before downloading when possible; never delete active artifacts |
| Range unsupported | Download the complete containing shard |
| Range response incorrect | Reject the artifact and retry another source or fallback |
| Hash or identity mismatch | Delete the invalid temporary artifact and fail preparation |
| GPU load fails | Keep valid disk cache, release GPU reservation, and fail preparation |
| One stage is not ready | Do not commit any stage |
| Provider publishes a new revision | Keep active deployment pinned; resolve the new revision only for a new deployment |
| Provider access missing or invalid | Reject preparation on that node; do not retry as a transient network error |
| Adapter version cannot serve pinned manifest | Fail preparation; recreate deployment after re-resolve |

## Capacity result

Partial weight placement allows the mesh to load a model larger than one node's usable memory when:

- Every required tensor is assigned.
- Global and tied weights are accounted for.
- Every node has runtime and KV-cache reserve.
- Selected nodes support the model operations.
- Selected nodes form a working direct pipeline.

It increases usable model capacity. It does not guarantee faster token generation.

## Crate ownership for P06

| Concern | Crate |
|---|---|
| Provider adapters, manifest generation, range planning, validation | `mesh-model` |
| Cache metadata, credentials, manifest rows | `mesh-store` |
| Disk/GPU leases during prepare | `mesh-inference` Local Resource Manager |
| Runtime wiring and GUI commands | `mesh-node`, `mesh-app` |
| Stable identity and plan types | `mesh-core` |

## Sources

- [Hugging Face Hub Rust client](https://github.com/huggingface/hf-hub)
- [Safetensors format](https://github.com/huggingface/safetensors)
- [Safetensors metadata and HTTP range example](https://huggingface.co/docs/safetensors/en/metadata_parsing)
