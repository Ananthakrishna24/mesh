# ADR-0007: Qwen3 4B and 8B Are the First Test Models

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-08-09 |
| Owners | Architecture discussion |

## Context

The first model family must prove complete-model inference, partial layer loading, provider downloads, Windows and Linux CUDA, macOS Metal, and a real distributed model-capacity gain.

A gated model would complicate onboarding. Separate 3B and 8B architectures would require two adapters before the first distributed proof.

## Decision

Use the dense Qwen3 family:

- `Qwen/Qwen3-4B` for single-node, backend, and fast smoke proofs.
- `Qwen/Qwen3-8B` for distributed layer-pipeline and larger-capacity proofs.

Use public Hugging Face Hub artifacts resolved to immutable revisions. Use upstream Safetensors without quantization for the first correctness path.

Use FP16 runtime weights and FP16 wire activations for the cross-platform baseline. Limit the first profile to 4,096 tokens, batch size 1, and non-thinking mode.

Canonical adapter contract: [Qwen3 dense model family](../architecture/inference/qwen3-model-family.md)

## Why 4B instead of an exact 3B model

Qwen3-4B and Qwen3-8B share the same dense architecture, Apache-2.0 license, public provider access, Safetensors format, and Candle Qwen3 implementation. One adapter serves both proofs.

Qwen2.5-3B uses a different Qwen research license and an older architecture. The exact parameter count is less valuable than reducing first-adapter scope.

## Why 8B

Qwen3-8B has 8.2B parameters and 36 layers. At 16-bit parameter storage, its weights alone are approximately 16.4 GB before KV cache and runtime overhead. This is large enough to exercise useful layer placement across common consumer GPUs.

## Rejected: first proof with only a tiny model

A tiny model can test token generation but may not prove partial provider download, meaningful memory placement, or a necessary multi-node pipeline.

Qwen3-4B remains small enough for repeated development while Qwen3-8B provides the capacity proof.

## Rejected: Qwen3 mixture-of-experts first

Mixture-of-experts adds expert routing, uneven layer memory, and conditional network paths. The first adapter uses the dense Qwen3 model only.

## Rejected: quantization before the correctness proof

Quantization adds backend-specific kernels and numerical behavior. First prove the unquantized stage boundary. Select one 4-bit format afterward.

## Candle status

Current Candle source contains dense Qwen3 configuration, model code, and explicit 4B and 8B example options. Candle remains subject to native Windows CUDA, Linux CUDA, and macOS Metal proofs.

The current complete Candle Qwen3 model constructs all layers. The mesh implements a stage-aware Qwen3 runtime that constructs only assigned layers. Loading the full model and discarding layers does not satisfy partial placement.

## Windows CI timing

Native Windows implementation and manual proofs remain required. Windows CI is introduced after the first confident native Windows implementation, targeted after the Qwen3-4B Windows CUDA proof. Delaying CI does not make Windows optional.

## Consequences

- Hugging Face Hub becomes the accepted first model provider.
- Qwen3 dense becomes the first Model Family Adapter.
- Qwen3-4B is the normal development model.
- Qwen3-8B is the distributed acceptance model.
- Thinking mode, long contexts, mixture-of-experts, and quantization remain later work.
- The adapter version becomes part of the Mesh Model Manifest cache key.

## Sources

- [Qwen3-4B](https://huggingface.co/Qwen/Qwen3-4B)
- [Qwen3-8B](https://huggingface.co/Qwen/Qwen3-8B)
- [Candle Qwen example](https://github.com/huggingface/candle/tree/main/candle-examples/examples/qwen)
- [Candle Qwen3 source](https://github.com/huggingface/candle/blob/main/candle-transformers/src/models/qwen3.rs)
