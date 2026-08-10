# KV-Cache Contract

| Field | Value |
|---|---|
| Status | Accepted |
| Canonical for | Per-stage KV-cache layout, dtype, context limits, GQA, allocation, cancellation, estimation, and eviction |
| Parent | [Distributed LLM inference](README.md) |
| Related | [Qwen3 dense model family](qwen3-model-family.md) |
| Related | [Tokenizer and sampling ownership](tokenizer-and-sampling.md) |
| Related | [Inference parallelism and edge cases](parallelism-and-edge-cases.md) |
| Decision | [ADR-0016: Tokenizer, sampling, and KV-cache contracts](../../decisions/0016-tokenizer-sampling-kv-cache.md) |
| Implements gate | A06 |

## Boundary

Each Inference Worker stage owns the key/value cache for **only** the transformer layers assigned to that stage. KV tensors never cross the network during normal generation.

This contract defines layout, data type, context limits, batching rules, grouped-query attention packing, sliding-window policy, cancellation, memory estimation, and eviction for the first correctness profile.

It does not define activation wire format ([Activation tensor frame](../protocol/activation-frame.md)) or sampling ownership ([Tokenizer and sampling](tokenizer-and-sampling.md)).

## Terms

- **Stage cache:** all KV storage for one stage role on one worker.
- **Request slot:** reserved KV capacity for one inference request inside a stage cache.
- **Layer cache:** K and V tensors for one decoder layer inside one request slot.
- **Sequence length (`seq_len`):** number of tokens already written into the slot (prompt tokens after prefill, plus generated tokens).
- **Context limit:** maximum `seq_len` the deployment allows; first profile hard-caps at **4,096**.
- **Batch dimension:** number of concurrent sequences sharing one fused kernel launch; first profile serving default is **1**.

## Ownership

| Concern | Owner |
|---|---|
| Bytes on device for assigned layers | Stage worker (`mesh-inference` / `mesh-compute`) |
| Reservation of KV bytes | Local Resource Manager at prepare/commit |
| Context limit and max concurrent slots | Placement plan / deployment |
| Request create, grow accounting, free | Stage runtime |
| Cross-node KV copy or migration | **Out of scope** (deferred; rejected for v1) |

Middle stages own only their layer range. First and final stages own only their layer ranges plus their non-KV weights (embeddings / head). A `Complete` stage owns every layer's KV.

## First correctness profile

| Setting | Value |
|---|---|
| Runtime KV dtype | FP16 (`f16`) |
| Context limit | 4,096 tokens |
| Default batch / concurrent sequences per deployment on one stage | 1 |
| Sliding window | Disabled; reject models that require a non-null sliding window for correctness |
| Live KV migration | No |
| KV over the wire | No |
| Paged / block allocator | Not required in v1; contiguous per-request buffers allowed |
| Prefix cache sharing across requests | No |
| Quantized KV | No |

Qwen3-4B and Qwen3-8B publish `max_position_embeddings = 40960` and `sliding_window = null`. The mesh still enforces the **4,096** profile cap for v1 proofs. YaRN and longer contexts stay deferred.

## Layout

### Logical layout per layer

For each assigned decoder layer `i` and each request slot:

```text
K_i: [batch, num_kv_heads, seq_capacity, head_dim]
V_i: [batch, num_kv_heads, seq_capacity, head_dim]
```

- Storage is contiguous for each of `K_i` and `V_i` separately.
- Dimension order is fixed as above for estimation and tests.
- Backend may use an equivalent packed buffer if it preserves element count, dtype, and head semantics.
- `batch` is the slot's batch size (1 in the first profile).
- `seq_capacity` is fixed at allocation time to the deployment context limit (4,096), not grown token-by-token with realloc, unless an implementation uses an internal arena that never exceeds the reserved byte cap.
- Valid tokens occupy `0 .. seq_len` along the sequence axis. Positions `seq_len .. seq_capacity` are unused capacity and must not be attended to.

### Stage cache structure

```text
Qwen3StageCache
├── deployment_id
├── stage_role
├── layer_range: [start, end)     // end exclusive
├── num_kv_heads
├── head_dim
├── context_limit
├── dtype: F16
└── slots[]
    ├── request_id
    ├── seq_len
    ├── seq_capacity
    ├── batch
    ├── state: Empty | Prefill | Decode | Complete | Cancelled
    └── layers[local_layer_index] → { K, V }
```

`local_layer_index` maps `0 .. layer_count_owned` onto the global `model.layers.*` range.

### Prefill vs decode writes

**Prefill**

1. Slot is created with `seq_len = 0`.
2. Prompt may arrive as one chunk or several causal chunks.
3. Each chunk appends keys/values at `seq_len` and then sets `seq_len += chunk_len`.
4. Attention for a chunk may only see positions `< new_seq_len` with standard causal masking.
5. Prefill fails if `seq_len + chunk_len > context_limit`.

**Decode**

1. Exactly one new token position is written per successful decode step.
2. `seq_len` becomes `seq_len + 1` after the write.
3. Decode fails if `seq_len + 1 > context_limit`.

Stages do not reorder writes from different transfer ids; they apply them in model order using request state and sequence position from the activation header / control path.

## Data type

| Item | Value |
|---|---|
| Stored K/V | IEEE 754 binary16 |
| Attention math | Backend-defined; may upcast to FP32 accumulators |
| Wire activations | FP16 (separate contract); not KV |
| Host staging | Allowed for load/debug; serving path keeps KV on device |

Do not store BF16 KV in the first profile even if weights arrived as BF16 from the provider. Weights are cast to the runtime FP16 profile at load; KV follows the runtime profile.

## Grouped-query attention (GQA)

Qwen3-4B and Qwen3-8B use:

| Field | Qwen3-4B | Qwen3-8B |
|---|---:|---:|
| `num_attention_heads` | 32 | 32 |
| `num_key_value_heads` | 8 | 8 |
| `head_dim` | 128 | 128 |
| `hidden_size` | 2,560 | 4,096 |

Rules:

1. K and V are stored with `num_kv_heads`, **not** `num_attention_heads`.
2. Query heads map onto KV heads by the standard repeat factor  
   `num_attention_heads / num_key_value_heads` (4 for both first models).
3. Estimation and allocation use `num_kv_heads` only.
4. If `num_attention_heads % num_key_value_heads != 0`, the adapter rejects the model.
5. Qwen3 QK-norm runs on queries/keys before cache write according to the model definition; cached K is post-projection (and post-k-norm when the reference model writes post-norm keys). Implementations must match the Candle/Qwen3 reference path chosen in P07 and keep that choice fixed for a given adapter version.

## Sliding window

| Rule | Value |
|---|---|
| First profile | No sliding-window attention |
| Config `sliding_window` | Must be null / absent / disabled |
| Non-conforming model | Reject at resolve/adapter time |
| Cache eviction along a window | Not used |

When a later profile enables sliding windows, it needs a new adapter version and an explicit cache ring-buffer rule. Until then, full causal context up to `context_limit` is retained for the life of the request.

## Maximum context handling

1. Deployment `context_limit` defaults to `4096` and cannot exceed `4096` in the first profile.
2. Planner and Local Resource Manager size KV reservations using that limit, not the provider's `max_position_embeddings`.
3. Coordinator rejects requests when `prompt_tokens + max_new_tokens > context_limit` ([tokenizer contract](tokenizer-and-sampling.md)).
4. Stage still hard-checks append bounds; overflow is a request error, not silent wrap.
5. RoPE positions use the absolute token position starting at 0 for the first prompt token. Positions must stay within the provider model limit; the mesh limit is the tighter 4,096 cap.

## Batch allocation

### First profile

- Serving allocations use `batch = 1` per request slot.
- A deployment may reserve `max_concurrent_requests` slots (default **1** for P07).
- Dynamic batching across requests is allowed later only when every batched request shares the deployment and the reserved KV bytes cover `sum(slot_bytes)` at full `context_limit` ([parallelism doc](parallelism-and-edge-cases.md)).
- v1 implementations may reject batch > 1 rather than partially implementing fused batching.

### Slot lifecycle

```text
reserve slot (Empty)
    → begin prefill (Prefill)
    → decode steps (Decode)
    → complete / cancel / error
    → free slot (device memory returned to stage pool or allocator)
```

Rules:

- One `request_id` owns at most one slot per stage.
- Slot creation fails fast with `RESOURCE_BUSY` when no free slot remains under the deployment reservation.
- Successful terminal states wipe K/V contents or return memory before the slot is reusable.
- Slots are not reused for a different `request_id` without full free.

## Memory estimation

### Per-layer KV bytes

```text
bytes_per_element = 2   // FP16

per_layer_kv_bytes =
    2                               // K and V
  * batch
  * num_kv_heads
  * seq_capacity
  * head_dim
  * bytes_per_element
```

### Per-request stage bytes

```text
request_stage_kv_bytes =
    per_layer_kv_bytes * layer_count_owned
```

### Deployment stage reserve

```text
stage_kv_reserve_bytes =
    request_stage_kv_bytes * max_concurrent_requests
  + allocator_overhead_bytes
```

`allocator_overhead_bytes` is implementation-defined but must be:

- non-negative,
- included in the Local Resource Manager reservation,
- stable for a backend once measured (document the constant or formula in code via named constants, not comments-only).

### Worked examples (batch=1, seq=4096, FP16)

**Qwen3-4B** (`num_kv_heads=8`, `head_dim=128`):

```text
per_layer = 2 * 1 * 8 * 4096 * 128 * 2 = 16,777,216 bytes  (16 MiB)
all 36 layers ≈ 576 MiB
```

**Qwen3-8B** (same GQA shape):

```text
per_layer = 16 MiB (identical KV geometry)
all 36 layers ≈ 576 MiB
```

A two-stage equal split of 18 layers each reserves ≈ 288 MiB KV per stage before overhead, plus weights and runtime work buffers outside this contract.

Planner must use resolved config values, not these examples alone.

### What estimation excludes

- Model weights
- Activation workspace / flash-attn work buffers
- CUDA/Metal context overhead
- Host RAM tokenizer state
- Disk cache

Those are reserved separately. KV reserve is only for K/V storage described above.

## Cancellation and failure

| Event | KV action |
|---|---|
| `CancelRequest` | Free request slot on every stage; drop queued activations for that id |
| EOS / max tokens / normal complete | Free slot after final token path finishes |
| Stage disconnect | Slot dies with the process; peer stages free on cancel/error propagation |
| Deployment release | Free every slot for that deployment |
| Reservation lease expiry | Free slots and refuse further appends |
| Prefill/decode error | Free slot; request ends with `error` |

Cancellation is synchronous with respect to new attention writes: after cancel is applied, no further K/V append for that `request_id` is allowed.

Partial prefill progress is discarded; v1 does not checkpoint KV to disk.

## Eviction policy

v1 eviction is **request-scoped only**:

1. Never evict KV for an active non-terminal request to make room for another request.
2. If a new request cannot obtain a free slot within the deployment reservation, reject it (`RESOURCE_BUSY`).
3. Do not page active KV to host under memory pressure in v1.
4. Do not steal capacity from another deployment's reservation.
5. After terminal state, memory returns to the stage's free pool for that deployment only.

Global GPU cache pressure is handled by refusing new reservations or new requests, not by silent KV drop.

## Attention mask and positions

- Full causal mask over `0 .. seq_len`.
- No alibi in Qwen3 first path; use RoPE as configured.
- Sequence position in the activation header is the first token position represented by that activation; KV write indices must agree.
- For decode activations with `sequence = 1`, the written KV index equals the previous `seq_len`.

## Interaction with placement

1. Placement computes `request_stage_kv_bytes` per stage from the layer assignment.
2. Resource offers include KV bytes inside the memory reservation request.
3. Commit fails if the Local Resource Manager cannot still hold the reserve.
4. Changing `context_limit` or `max_concurrent_requests` requires a new deployment; live resize is forbidden.

## Explicit non-goals (v1)

- Live KV replication or stage migration.
- Cross-request prefix caching.
- Paged KV as a required allocator (optional internal impl detail only if reserved bytes stay exact).
- INT8/FP8 KV.
- Host-pinned KV serving path as default.
- Sliding-window ring caches.
- Speculative-decoding tree caches.

## Crate ownership

| Concern | Crate |
|---|---|
| Estimation pure functions and limits | `mesh-core` or `mesh-inference` (pure, no GPU) |
| Slot table and lifecycle | `mesh-inference` |
| Device buffers and attention kernels | `mesh-compute` |
| Reservation amounts | `mesh-inference` Local Resource Manager |
| Activation ordering vs `seq_len` | `mesh-inference` + `mesh-net` validation |

## Acceptance checks for P07 / P09

- Complete-stage Qwen3-4B allocates KV using the formula above within a measured tolerance of allocator overhead.
- Prefill then decode increases `seq_len` exactly by prompt length then +1 per token.
- Cancel frees device memory enough for a subsequent identical request to allocate again.
- Two-stage proof: each stage's layer count matches its KV stack depth; no stage stores the other's layers.
- Overflow request (`prompt + max_new > 4096`) never creates a slot.
- GQA storage uses 8 KV heads, not 32, for both first models.
