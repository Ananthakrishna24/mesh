# Tokenizer and Sampling Ownership

| Field | Value |
|---|---|
| Status | Accepted |
| Canonical for | Tokenizer ownership, chat template, sampling state, next-token selection, and token streaming |
| Parent | [Distributed LLM inference](README.md) |
| Related | [Qwen3 dense model family](qwen3-model-family.md) |
| Related | [Control protocol](../protocol/control-protocol.md) |
| Decision | [ADR-0016: Tokenizer, sampling, and KV-cache contracts](../../decisions/0016-tokenizer-sampling-kv-cache.md) |
| Implements gate | A05 |

## Boundary

This contract fixes who owns tokenization, detokenization, prompt formatting, sampling parameters, sampling RNG state, next-token selection, token history for penalties, and end-of-sequence decisions.

It does not define KV-cache layout. That lives in [KV-cache contract](kv-cache.md).

It does not define activation wire bytes. That lives in [Activation tensor frame](../protocol/activation-frame.md).

## Ownership summary

| Concern | Owner | Rationale |
|---|---|---|
| Tokenizer bytes and `tokenizer_hash` check | Coordinator | One text identity for the deployment; GUI and logs stay on the job owner |
| Chat template and non-thinking prompt assembly | Coordinator | User-visible prompt formatting must not diverge across stages |
| Prompt → token IDs | Coordinator | Stages receive token IDs, never raw user text, on the first path |
| Detokenization and streamed text | Coordinator | Users see one decode path; partial UTF-8 is handled once |
| Embedding lookup | First stage (`First` or `Complete`) | Embeddings are model weights owned by the first continuous range |
| Output head / `lm_head` (or tied embedding projection) | Final stage (`Final` or `Complete`) | Logits exist only where the head runs |
| Sampling parameters for a request | Coordinator declares; final stage enforces | Parameters travel with `InferenceRequest` |
| Sampling RNG state | Final stage | Avoids multi-hop RNG sync; seed is set once per request |
| Token history used by penalties | Final stage | History is local to the sampler that needs it |
| Next-token selection | Final stage | One place applies temperature, top-k, top-p, penalties, and EOS |
| Next token ID for continued decode | Final stage → first stage | Control-path token ID, not an activation tensor |
| Generated token result for the user | Final stage → coordinator | `TokenResult` carries ids and optional detok metadata only when needed |
| Stop decision (EOS, max new tokens, cancel) | Final stage decides; coordinator may cancel | Final stage stops sampling; coordinator remains authoritative for user cancel |

A single-node `Complete` stage still obeys the same logical split inside one process: coordinator code tokenizes and streams; the complete worker embeds, runs layers, samples, and returns tokens.

## Terms

- **Tokenizer identity:** the exact tokenizer artifact set and `tokenizer_hash` from the Mesh Model Manifest.
- **Prompt tokens:** token IDs produced by the coordinator after chat-template rendering.
- **Generated tokens:** token IDs produced by the final stage after the prompt.
- **Token history:** ordered token IDs available to repetition penalties (prompt + generated, subject to the rules below).
- **Sampling profile:** named default parameter set; first profile is Qwen3 non-thinking.
- **Request sampling override:** per-request parameters clamped to deployment limits.
- **Stop reason:** why generation ended for one request.

## Tokenizer ownership

### Coordinator duties

1. Load tokenizer artifacts from the local Model Store using the deployment's pinned revision and `tokenizer_hash`.
2. Reject deployment start when local tokenizer bytes do not match `tokenizer_hash`.
3. Render the Qwen3 non-thinking chat template for the first proofs.
4. Encode the rendered prompt to token IDs with the pinned tokenizer.
5. Reject requests whose prompt token count exceeds the deployment context budget after reservation of `max_new_tokens`.
6. Send `InferenceRequest` with token IDs (not raw UTF-8 prompt text) to the first stage.
7. Detokenize generated token IDs for the GUI and API surface.
8. Preserve incomplete UTF-8 sequences across streamed tokens until a complete character boundary exists.
9. Own user-visible stop reasons and final request status.

### Stage duties

- First stage accepts token IDs and runs embeddings. It does not re-tokenize user text.
- Final stage does not detokenize for the user path. It may keep token IDs only.
- Workers may load tokenizer artifacts only for optional local diagnostics. Diagnostics must not change serving behavior or bypass `tokenizer_hash`.

### Tokenizer artifacts

Required set matches the model distribution contract:

- `tokenizer.json` and every sidecar it references
- optional chat/template files required by the non-thinking profile

`tokenizer_hash` is SHA-256 over the exact artifact bytes used, as defined in [Provider-backed model distribution](model-distribution.md).

### Chat template — first profile

| Rule | Value |
|---|---|
| Mode | Qwen3 non-thinking only |
| Thinking mode | Disabled; reject requests that ask to enable it in v1 |
| Roles | System (optional), user, assistant |
| Multi-turn | Allowed when the full rendered prompt fits the context budget |
| Special tokens | Exactly those emitted by the pinned tokenizer/template; do not hand-invent IDs |
| BOS | Follow tokenizer/template; do not double-insert |
| Generation prompt | Template must end ready for assistant generation |

Default development system prompt may be empty. Tests that need stable strings pin both system and user text in the test.

## Request and control flow

### Messages already reserved

From the [control protocol](../protocol/control-protocol.md):

- `InferenceRequest` (field 40)
- `TokenResult` (field 41)
- `CancelRequest` (field 42)

### Logical `InferenceRequest` fields

Exact protobuf fields are added in the P07 schema change. Semantically required content:

| Field | Rule |
|---|---|
| `deployment_id` | 16 bytes; accepted deployment |
| `request_id` | 16 bytes; unique per request |
| `input_token_ids` | `u32` token IDs; prompt only |
| `max_new_tokens` | `u32`; required; ≥ 1 |
| `temperature` | `f32`; see sampling rules |
| `top_k` | `u32`; `0` means disabled |
| `top_p` | `f32`; see sampling rules |
| `repetition_penalty` | `f32`; see sampling rules |
| `presence_penalty` | `f32`; v1 fixed at `0.0` unless explicitly enabled later |
| `frequency_penalty` | `f32`; v1 fixed at `0.0` unless explicitly enabled later |
| `seed` | `u64`; required for every request in the first profile |
| `stop_token_ids` | optional extra stop IDs; EOS from config always stops |
| `return_logprobs` | bool; v1 must be `false` |

Coordinator → first stage for pipeline and complete modes. In replica mode the coordinator sends the request to the assigned complete replica worker.

### Logical `TokenResult` fields

| Field | Rule |
|---|---|
| `deployment_id` | 16 bytes |
| `request_id` | 16 bytes |
| `token_id` | `u32` |
| `token_index` | `u32`; 0-based index in the generated stream |
| `is_last` | bool |
| `stop_reason` | set when `is_last` is true; unset otherwise |
| `sequence_length` | total tokens in KV after this step (prompt + generated) |

Final stage → coordinator on every accepted token, including the final token that carries the stop reason.

When `is_last` is true because of EOS, the EOS token itself is still delivered once unless the deployment is configured to suppress it. The first profile **delivers** the EOS token id and marks `is_last`.

### Next-token feedback to the first stage

After each sampled token that is not last:

1. Final stage sends the token ID to the first stage on the control path (same logical request).
2. First stage embeds that single token and runs decode for its layer range.
3. Activations continue along the pipeline as in the activation-frame contract.

The feedback message is a control message scoped to the deployment/request. It is not an activation tensor and must not use the activation frame.

For a `Complete` stage, feedback is an in-process call.

### Cancellation

- User or coordinator cancel uses `CancelRequest`.
- Final stage stops sampling as soon as cancel is observed.
- First and middle stages drop queued work for that request and release request-scoped KV (see KV-cache contract).
- Coordinator emits a terminal user status `cancelled` even if a racing `TokenResult` arrives; late tokens after terminal status are ignored.

## Sampling profile

### Default non-thinking profile (Qwen3)

| Parameter | Default | Notes |
|---|---|---|
| Temperature | `0.7` | |
| Top-p | `0.8` | Nucleus sampling |
| Top-k | `20` | Applied before top-p |
| Repetition penalty | `1.0` | Disabled at `1.0` |
| Presence penalty | `0.0` | Not applied in v1 |
| Frequency penalty | `0.0` | Not applied in v1 |
| Seed | Required | Caller or test supplies; coordinator may assign a random `u64` when the UI leaves it unset |
| Max context | `4096` tokens | Hard deployment cap for the first correctness profile |
| Thinking | Off | |

Defaults match the Qwen3 model-family contract. A request may override temperature, top-k, top-p, repetition penalty, seed, and `max_new_tokens` within the clamps below.

### Clamps and validation

| Parameter | Valid range | On violation |
|---|---|---|
| `temperature` | `0.0 ..= 2.0` | Reject request |
| `top_k` | `0` or `1 ..= vocab_size` | Reject request |
| `top_p` | `0.0 ..= 1.0` | Reject request |
| `repetition_penalty` | `0.1 ..= 2.0` | Reject request |
| `max_new_tokens` | `1 ..= remaining_context` | Reject request |
| `seed` | any `u64` | Required; missing seed rejects in first profile |
| Prompt length | `>= 1` and `prompt + max_new_tokens <= context_limit` | Reject request |

`remaining_context = context_limit - prompt_token_count`.

### Greedy path

When `temperature == 0.0`:

1. Ignore top-k and top-p.
2. Apply repetition penalty to logits if `repetition_penalty != 1.0`.
3. Choose `argmax`.
4. Ties break by lowest token id.
5. RNG is not advanced.

### Temperature > 0 sampling algorithm

Operate on the final-stage logits vector `L[0..vocab)` in FP32 working precision (logits may be produced in FP16 and upcast for sampling):

1. **Repetition penalty** (if `repetition_penalty != 1.0`):
   - Let `H` be the token-history multiset defined below.
   - For each distinct token id `t` present in `H`:
     - If `L[t] > 0`: `L[t] /= repetition_penalty`
     - Else: `L[t] *= repetition_penalty`
   - This is the Hugging Face-style signed repetition penalty.
2. **Temperature:** `L[t] /= temperature`.
3. **Top-k** (if `top_k > 0`): keep the `k` largest logits; set all others to `-∞`.
4. **Top-p** (if `top_p < 1.0`):
   - Softmax the finite logits into probabilities.
   - Sort by probability descending; ties break by lower token id.
   - Keep the smallest prefix whose cumulative probability is ≥ `top_p`, always keeping at least the first token.
   - Zero the remaining probabilities and renormalize.
5. If top-p was not applied, softmax the finite logits.
6. Draw one token from the discrete distribution using the request RNG.
7. If every probability is non-finite or the distribution is empty after filters, fail the request with `REQUEST_REJECTED` / internal sampling failure; do not silently pick token 0.

Order is fixed: **penalty → temperature → top-k → top-p → sample**. Implementations must not reorder these steps.

### Random seed and RNG

| Rule | Value |
|---|---|
| Seed scope | One request |
| RNG owner | Final stage |
| Algorithm | `rand_chacha::ChaCha12Rng` seeded with the little-endian `u64` seed |
| Draw method | After probabilities are final, use a single `u64` draw mapped with unbiased `f64` selection over the CDF (or equivalent one-draw categorical sampling that does not introduce extra draws per rejected candidate) |
| Greedy | RNG is not used and not advanced |
| Resume | v1 does not resume mid-request RNG; cancel or failure restarts from the prompt with a new request id |
| Cross-backend identity | Same seed does **not** guarantee identical tokens across CUDA vs Metal because logits may differ; same backend + same binary profile should be deterministic for tests |

Determinism requirements for CI:

- Same machine class, same backend, same model revision, same request body, temperature 0 → identical token ids.
- Temperature > 0 tests assert distribution constraints or fix temperature 0 for exact strings.

### Token history for penalties

| Rule | Value |
|---|---|
| Included tokens | All prompt token IDs, then each generated token after it is accepted |
| When history updates | After a token is selected and before the next sampling step |
| EOS in history | If EOS is produced, it is appended; sampling then stops |
| Window | Entire request history up to `context_limit` (no frequency-window trimming in v1) |
| Storage | Final stage only; not shipped on the activation path |
| Coordinator copy | Coordinator keeps generated ids for detokenization; that copy is not the penalty authority |

Presence and frequency penalties are defined as no-ops at `0.0` in v1 so the wire fields can exist later without behavior change.

### End-of-sequence and stop rules

Stop when any of the following is true, checked after each sampled token:

1. Sampled token id is the model `eos_token_id` from resolved `config.json` (Qwen3-4B/8B: `151645`).
2. Sampled token id is in the request `stop_token_ids` list.
3. Generated count reaches `max_new_tokens`.
4. `prompt_len + generated_len` would exceed `context_limit` on the next step.
5. Cancel is observed.
6. Deployment or stage failure.

Stop reasons:

| Reason | Meaning |
|---|---|
| `eos` | Model or request stop token |
| `max_new_tokens` | Hit request length limit |
| `context_limit` | Hit deployment context cap |
| `cancelled` | User or coordinator cancel |
| `error` | Failure path |

Only one terminal reason is reported. Priority if multiple apply on the same step: `cancelled` > `error` > `eos` > `max_new_tokens` > `context_limit`.

### Vocabulary and invalid ids

- Vocabulary size comes from the resolved model config / manifest.
- Sampler output must always be in `0 .. vocab_size`.
- First stage rejects out-of-range token IDs on `InferenceRequest` or feedback with `REQUEST_REJECTED`.
- Softmax and sampling never invent ids outside the vocab.

## Streaming to the GUI

1. Coordinator receives each `TokenResult`.
2. Appends `token_id` to the request's generated id list.
3. Detokenizes incrementally; emits only complete UTF-8 characters to the UI snapshot.
4. On `is_last`, flushes any remaining decoder state per tokenizer rules and marks the request finished with `stop_reason`.
5. UI never samples or tokenizes.

## Warm-up request

Deployment warm-up uses the same ownership rules:

- Coordinator builds a short fixed prompt (implementation-defined, documented in tests).
- Temperature `0.0`, `max_new_tokens` small (≤ 8), fixed seed.
- Tokens may be discarded after success; they still exercise tokenize → embed → layers → sample → stream paths.
- Warm-up failure blocks commit.

## Single-node vs pipeline vs replica

| Mode | Tokenizer | Embeddings | Sampler |
|---|---|---|---|
| Single-node complete | Coordinator process | Complete worker | Complete worker |
| Replica | Coordinator process | Assigned replica worker | Same worker |
| Layer pipeline | Coordinator process | First stage worker | Final stage worker |

Middle stages never tokenize or sample.

## Explicit non-goals (v1)

- Speculative decoding and draft models.
- Returning full logprobs or top-n alternatives to the client.
- Worker-side re-tokenization of user text.
- Thinking-mode templates.
- Beam search.
- Best-of-n sampling.
- Cross-node RNG synchronization.
- Mid-request sampler state migration.
- Detokenization on the final stage for user output.

## Crate ownership

| Concern | Crate |
|---|---|
| Request/result types and validation | `mesh-core` |
| Control send/receive | `mesh-net` |
| Coordinator tokenize/detokenize/template/stream assembly | `mesh-inference` |
| Final-stage sampler and RNG | `mesh-inference` |
| Embedding and LM head execution | `mesh-compute` via stage runtime |
| GUI display of streamed text | `mesh-app` over snapshots only |

## Acceptance checks for P07

- Coordinator-only tokenization: workers do not need user text.
- Seeded temperature-0 Qwen3-4B run is repeatable on one backend.
- Default non-thinking profile values match this document unless the request overrides them.
- Cancel stops further `TokenResult` acceptance on the coordinator.
- EOS and max-new-tokens each produce the correct stop reason.
- Pipeline path samples only on the final stage and feeds token ids back to the first stage.
