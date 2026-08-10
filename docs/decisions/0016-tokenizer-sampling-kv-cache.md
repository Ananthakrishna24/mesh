# ADR-0016: Tokenizer, Sampling, and KV-Cache Contracts

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-08-10 |
| Owners | Architecture discussion |
| Gates | A05, A06 |

## Context

P07 single-node inference and later pipeline modes need fixed answers for:

- Who tokenizes, detokenizes, samples, and stops a request (A05).
- How each stage stores, sizes, and frees KV cache (A06).

Earlier docs already preferred coordinator tokenization and final-stage sampling, and stated that each stage keeps KV for its own layers. Those preferences were not locked with algorithms, clamps, message semantics, GQA math, or eviction rules. Without that lock, P07 would invent incompatible sampler orderings and KV layouts across backends.

## Decision

Accept the detailed contracts:

- [Tokenizer and sampling ownership](../architecture/inference/tokenizer-and-sampling.md) for gate **A05**
- [KV-cache contract](../architecture/inference/kv-cache.md) for gate **A06**

### A05 — Tokenizer and sampling

- Coordinator owns tokenizer bytes (`tokenizer_hash`), non-thinking chat template, encode/decode, and user-visible streaming.
- First stage owns embeddings; final stage owns output head, sampling RNG, token history for penalties, and next-token selection.
- Stages receive token IDs, not raw user text, on the serving path.
- Default non-thinking profile: temperature `0.7`, top-p `0.8`, top-k `20`, repetition penalty `1.0`, required `u64` seed, context `4096`, thinking off.
- Sampling order is fixed: repetition penalty → temperature → top-k → top-p → sample. Temperature `0` is greedy argmax with lowest-id tie-break and does not advance RNG.
- RNG is `ChaCha12` on the final stage, seeded per request. Cross-backend token identity is not required when logits differ.
- Final stage streams `TokenResult` to the coordinator and feeds the next token ID to the first stage on the control path.
- Stop reasons: `eos`, `max_new_tokens`, `context_limit`, `cancelled`, `error`.

### A06 — KV cache

- Each stage stores FP16 K/V only for its assigned layers:  
  `[batch, num_kv_heads, seq_capacity, head_dim]` per K and V.
- First profile: context `4096`, batch `1`, no sliding window, no live migration, no KV on the wire, no cross-request prefix cache.
- GQA uses `num_kv_heads` (8 for Qwen3-4B/8B), not query head count.
- Memory estimate:  
  `2 * batch * num_kv_heads * seq_capacity * head_dim * 2 * layer_count_owned * max_concurrent_requests` (+ explicit allocator overhead).
- Eviction is request-scoped free only; never drop active KV to admit another request.
- Cancel, complete, error, deployment release, and lease expiry free the slot.

## Rejected: tokenizer on every stage

Re-tokenizing on workers risks hash drift, divergent templates, and inconsistent partial UTF-8 streaming. One coordinator tokenizer matches one `tokenizer_hash` and one UI decode path.

## Rejected: coordinator-side sampling

Shipping full logits to the coordinator each decode step is a large WAN payload and couples sampling to coordinator GPU absence. Final-stage sampling keeps logits local; only token IDs return.

## Rejected: shared multi-node KV or live migration in v1

Moving KV across peers needs ordering, bandwidth, and failure rules that the first proofs do not require. Stage loss restarts from the prompt.

## Rejected: sliding-window and long-context YaRN in v1

Qwen3 configs allow large `max_position_embeddings`, but the accepted correctness profile caps at 4,096 full-causal tokens. Windowed caches and YaRN wait for a later adapter version.

## Rejected: temperature/top-p order ambiguity

Different libraries disagree on filter order. Locking penalty → temperature → top-k → top-p removes cross-implementation drift inside mesh.

## Rejected: mandatory paged KV allocator in v1

Contiguous per-request buffers are enough for batch 1 and 4,096 context. Paging may appear later as an implementation detail only if reserved byte caps stay honest.

## Consequences

- P07 may implement Qwen3 tokenizer, chat template, seeded sampling, and stage KV against these contracts.
- Control schema work in P07 must carry the logical `InferenceRequest` / `TokenResult` fields described in the tokenizer contract; field numbers stay in the reserved 40–42 range family.
- Placement and P05 memory offers must include KV reserve bytes from the A06 formula before commit.
- Checklist gates A05 and A06 are resolved; A01/A02 remain implementation/profile locks during P07.
- Cross-backend golden strings stay temperature-0 and tolerance-based where logits diverge.
