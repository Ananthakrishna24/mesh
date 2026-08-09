# Provider-Backed Model Distribution

| Field | Value |
|---|---|
| Status | Accepted architecture; Hugging Face is the accepted first adapter |
| Canonical for | Resolving, partitioning, downloading, caching, and synchronizing model weights |
| Parent | [Distributed LLM inference](README.md) |
| Decision | [ADR-0004: immutable provider-backed model distribution](../../decisions/0004-provider-backed-model-distribution.md) |

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
    provider: String,
    repository: String,
    revision: String,       // immutable provider commit or content revision
    manifest_hash: String,
    model_format: ModelFormat,
    quantization: Option<String>,
    tokenizer_hash: String,
}
```

Never run a deployment from a moving name such as `main` alone. Resolve it once, pin the returned immutable revision, and send that revision to every node.

If the provider changes `main`, active deployments continue using their pinned revision. A new revision creates a new deployment.

## Mesh Model Manifest

The manifest normalizes provider files into model parts.

It contains:

- Immutable model identity.
- Architecture and configuration.
- Number of layers and hidden size.
- Supported data types and quantization.
- Tokenizer artifacts.
- Global tensors such as embeddings, final normalization, and output head.
- Tensor name, shape, data type, layer ownership, source artifact, and byte range.
- Whole-artifact ETag or digest when available.
- Per-tensor or range digest when available.
- Loaded-memory estimate.
- Runtime and operation requirements.

The manifest hash is part of every placement and readiness message.

## First provider

Use a Hugging Face Hub adapter first. The initial public references are `Qwen/Qwen3-4B` and `Qwen/Qwen3-8B`; every deployment pins an immutable revision before placement.

The Rust `hf-hub` client supports:

- Exact `revision` values for file download.
- Asynchronous downloads.
- Local caching.
- Repository snapshots.
- Authentication token discovery.
- Resumable internal file transfer.

Use the high-level client for metadata, complete files, and normal cache integration. Use an explicit HTTP range path only for tensor ranges after the immutable revision and artifact metadata are resolved.

The provider abstraction must not expose Hugging Face types to inference planning.


First model mapping: [Qwen3 dense model family](qwen3-model-family.md)

## Partial weight strategies

Use the first strategy that produces verified assigned tensors efficiently.

### 1. Local tensor cache

If the exact model revision and assigned tensor set already exist locally, reuse them.

### 2. Layer-aligned provider artifact

Download complete files when the provider already stores the assigned layers in their own artifact. This is the simplest partial distribution.

### 3. Safetensors byte ranges

Safetensors stores an eight-byte header length, a JSON header, and tensor bytes. The header maps tensor names to data offsets.

For each required tensor:

1. Fetch the first eight bytes.
2. Fetch and parse the JSON header.
3. Find the tensor's data offsets.
4. Convert those offsets to complete file offsets.
5. Merge nearby ranges to avoid many small requests.
6. Fetch only those ranges when the provider supports HTTP Range.
7. Validate response ranges, lengths, data types, and shapes.
8. Store the result under the immutable model and tensor identity.

Safetensors is the first accepted format for true tensor-level partial download.

### 4. Complete provider shard fallback

If byte ranges are unavailable, download every complete shard containing at least one assigned tensor. Keep only the tensors required at runtime, but preserve the valid shard in the disk cache.

This uses more disk and network data but keeps the model usable.

### 5. Verified peer cache

A connected peer may offer an exact artifact or tensor range it already has. The receiver checks immutable identity, length, and available digest before accepting it.

Peer cache sharing is an optimization. The provider remains the first source until peer transfer and cache inventory are measured.

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

## Cache behavior

Cache keys include immutable revision, artifact identity, byte range or tensor identity, data type, and quantization.

Rules:

- Write downloads to an incomplete temporary path.
- Rename or mark ready only after validation.
- Never serve incomplete data to an Inference Worker.
- Reuse valid data across deployments.
- Track last use and current deployment references.
- Evict only unreferenced entries.
- Prefer evicting old full shards when verified layer tensors remain.
- Rate-limit background cache work during active inference.

## Provider credentials

Provider credentials remain local to each node. The coordinator sends model identity and artifact references, not another node's access token.

For gated models, each selected node must already have valid provider access. A node without access rejects preparation before commit. The first version does not distribute credentials between peers.

Canonical storage contract: [Persistent state](../system/persistent-state.md)

## Failure handling

| Failure | Required behavior |
|---|---|
| Provider unavailable | Retry within the reservation lease, try a verified peer source, or reject preparation |
| Node leaves during download | Release the pending plan and create a new placement |
| Disk space exhausted | Reject before downloading when possible; never delete active artifacts |
| Range unsupported | Download the complete containing shard |
| Range response incorrect | Reject the artifact and retry another source |
| Hash or identity mismatch | Delete the invalid temporary artifact and fail preparation |
| GPU load fails | Keep valid disk cache, release GPU reservation, and fail preparation |
| One stage is not ready | Do not commit any stage |
| Provider publishes a new revision | Keep active deployment pinned; resolve the new revision only for a new deployment |

## Capacity result

Partial weight placement allows the mesh to load a model larger than one node's usable memory when:

- Every required tensor is assigned.
- Global and tied weights are accounted for.
- Every node has runtime and KV-cache reserve.
- Selected nodes support the model operations.
- Selected nodes form a working direct pipeline.

It increases usable model capacity. It does not guarantee faster token generation.

## Sources

- [Hugging Face Hub Rust client](https://github.com/huggingface/hf-hub)
- [Safetensors format](https://github.com/huggingface/safetensors)
- [Safetensors metadata and HTTP range example](https://github.com/huggingface/safetensors/blob/main/docs/source/metadata_parsing.mdx)
