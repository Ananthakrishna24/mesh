# Qwen3 Dense Model Family

| Field | Value |
|---|---|
| Status | Accepted first model family; not implemented |
| Canonical for | First model references, stage construction, runtime profile, and model proofs |
| Parent | [Distributed LLM inference](README.md) |
| Decision | [ADR-0007: Qwen3 4B and 8B](../../decisions/0007-qwen3-first-model-family.md) |

## Selected models

| Use | Provider reference | Parameters | Layers | License |
|---|---|---:|---:|---|
| Single-node smoke and backend proof | `Qwen/Qwen3-4B` | 4.0B | 36 | Apache-2.0 |
| Distributed capacity proof | `Qwen/Qwen3-8B` | 8.2B | 36 | Apache-2.0 |

Both are public dense Qwen3 models published as Safetensors on Hugging Face Hub. The Model Resolver converts `main` to an immutable provider revision before downloading any weights.

Qwen3-4B is the closest selected model to the requested 3B class. Using 4B and 8B keeps both tests on one model architecture and one Model Family Adapter.

## Why this family

- Public artifacts do not require gated-model approval.
- Apache-2.0 license.
- Safetensors and sharded-index support.
- One dense architecture at both useful sizes.
- Current Candle source includes dense Qwen3 model support and explicit 4B and 8B examples.
- Candle exposes CUDA and Metal device paths.
- The 8B unquantized model is large enough to exercise real multi-node layer placement on common consumer GPUs.

This is the best fit for the first system proof. It is not a claim that Qwen3-8B is the best model for every user workload.

## Excluded variants

- Do not use Qwen3 mixture-of-experts for the first adapter.
- Do not use a gated model for the first onboarding proof.
- Do not add GGUF or several quantization formats to the first correctness path.
- Do not enable YaRN or contexts above the initial limit.

## First correctness profile

| Setting | Value |
|---|---|
| Provider | Hugging Face Hub |
| Source format | Upstream Safetensors |
| Source precision | Provider precision, normally BF16 |
| Runtime weight dtype | FP16 for the cross-platform baseline |
| Activation wire dtype | FP16 |
| Maximum context | 4,096 tokens |
| Initial batch | 1 request |
| Thinking mode | Disabled |
| Sampling | Seeded non-thinking profile |
| Quantization | None |

FP16 is the initial cross-platform runtime format. It is supported broadly by NVIDIA CUDA and Apple Metal hardware. A later benchmark may allow BF16 when every selected stage proves support.

Approximate parameter storage before runtime and KV-cache overhead:

- Qwen3-4B at 16 bits: about 8 GB.
- Qwen3-8B at 16 bits: about 16.4 GB.

The planner uses actual tensor metadata, not these rounded estimates.

## Model configuration

The adapter reads `config.json` and validates at least:

- `model_type` identifies Qwen3.
- Vocabulary size.
- Hidden size.
- Intermediate size.
- Number of hidden layers.
- Number of attention heads.
- Number of key/value heads.
- Head dimension.
- RoPE parameters.
- RMS normalization epsilon.
- Tied embedding setting.
- Sliding-window settings.
- Maximum position embeddings.

The first profile rejects unsupported sliding-window or YaRN configurations instead of silently changing behavior.

## Tensor ownership

The normalized manifest maps these logical groups:

```text
First stage
├── model.embed_tokens.*
└── model.layers.0 .. model.layers.N

Middle stage
└── model.layers.N+1 .. model.layers.M

Final stage
├── model.layers.M+1 .. model.layers.35
├── model.norm.*
└── lm_head.* or tied embedding weights
```

Each decoder layer includes:

```text
model.layers.<index>
├── self_attn
│   ├── q_proj
│   ├── k_proj
│   ├── v_proj
│   ├── o_proj
│   ├── q_norm
│   └── k_norm
├── mlp
│   ├── gate_proj
│   ├── up_proj
│   └── down_proj
├── input_layernorm
└── post_attention_layernorm
```

Exact tensor names come from the resolved Safetensors headers. The adapter fails when required tensors are missing or unexpected model structure changes.

## Stage runtime

Implement a mesh-owned `Qwen3Stage`. It must load only its assigned components.

```rust
struct Qwen3Stage {
    role: StageRole,
    layer_range: Range<usize>,
    embedding: Option<Embedding>,
    layers: Vec<Qwen3DecoderLayer>,
    final_norm: Option<RmsNorm>,
    lm_head: Option<Linear>,
    kv_cache: Qwen3StageCache,
}
```

Roles:

- `First`: accepts token IDs, runs embedding and the first layer range.
- `Middle`: accepts and returns hidden activations.
- `Final`: runs the final layers, normalization, output head, and sampling.
- `Complete`: contains every component for one-node and replica inference.

The current Candle `ModelForCausalLM` constructs the complete model. The mesh adapter must not load the complete model and discard unused layers. It must construct only assigned layers from the selected tensor ranges.

## KV cache

Each stage stores keys and values only for its assigned layers. Cache sizing reads Qwen3 grouped-query attention fields from the resolved configuration.

The first profile supports:

- Batch size 1.
- Maximum 4,096 tokens.
- No live cache migration.
- Full cache release on request completion, cancellation, or stage failure.

Dynamic batching and larger contexts come after the basic distributed proof.

## Prompt and sampling profile

Use Qwen3 non-thinking chat formatting for the first proofs. Thinking mode increases output length and makes latency tests less predictable.

Default non-thinking sampling follows the model guidance where implemented:

- Temperature `0.7`.
- Top-p `0.8`.
- Top-k `20`.
- Fixed seed for repeatable test runs.

The final stage owns sampling. The coordinator owns tokenization, detokenization, and user-visible streaming.

## Required proofs

### Proof 1 — Complete 4B model

Run `Qwen/Qwen3-4B` independently on:

1. Native Windows x64 NVIDIA CUDA.
2. Linux x64 NVIDIA CUDA.
3. macOS Apple Silicon Metal.

Verify successful model resolution, cache, load, warm-up, prompt processing, decode, cancellation, and restart.

### Proof 2 — Partial 4B stages

On one development machine, split Qwen3-4B into at least two stage objects. Verify each stage loads only assigned tensors and the combined output matches the complete-stage path within the accepted numerical tolerance.

### Proof 3 — Distributed 8B model

Run `Qwen/Qwen3-8B` across at least two directly connected PCs. Every node downloads only its assigned tensor ranges or complete containing shards. Verify:

- Same immutable revision and manifest hash.
- Different continuous layer assignments.
- Local KV cache ownership.
- FP16 activation transfer.
- Token streaming from final stage to coordinator.
- Truthful failure when one stage disconnects.

A developer-only placement override may force two stages when test hardware could fit the complete model. This override is not the normal user planner.

### Proof 4 — Mixed backend route

Run a supported route containing at least two of:

- Windows CUDA.
- Linux CUDA.
- macOS Metal.

Compare stage-boundary tensor shapes, finite values, and final output behavior. Do not require identical generated tokens across different GPU backends when floating-point differences change close sampling scores.

## Quantized follow-up

After the unquantized 8B distributed proof, select exactly one 4-bit format. It must pass native Windows CUDA, Linux CUDA, and macOS Metal operation checks before becoming a planner capability.

Quantization is an optimization and capacity extension. It does not block the first correct layer pipeline.

## Sources

- [Qwen3-4B model card](https://huggingface.co/Qwen/Qwen3-4B)
- [Qwen3-8B model card](https://huggingface.co/Qwen/Qwen3-8B)
- [Candle Qwen example](https://github.com/huggingface/candle/tree/main/candle-examples/examples/qwen)
- [Candle Qwen3 implementation](https://github.com/huggingface/candle/blob/main/candle-transformers/src/models/qwen3.rs)
