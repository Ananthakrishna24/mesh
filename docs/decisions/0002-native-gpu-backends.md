# ADR-0002: Native CUDA and Metal Backend Paths

| Field | Value |
|---|---|
| Status | Accepted boundary; libraries proposed |
| Date | 2026-08-09 |
| Owners | Architecture discussion |

## Context

The first GPU targets are NVIDIA CUDA on Windows and Linux and Apple Metal on macOS. The mesh needs one way to describe hardware and schedule work, but the platforms expose different native runtimes and performance libraries.

## Decision

Keep one platform-neutral Rust boundary with two native implementations:

- CUDA for NVIDIA GPUs on native Windows and Linux.
- Metal for Apple GPUs on macOS Apple Silicon.

Separate hardware discovery from compute execution.

Use Candle as the first framework to evaluate for inference because it exposes both CUDA and Metal features from Rust. Do not select the distributed training framework until a real training workload is defined and measured.

Canonical details: [GPU backends](../architecture/compute/gpu-backends.md)

Required Windows target: [ADR-0006](0006-windows-nvidia-required.md)

## Rejected: one `wgpu` execution path for every GPU

A single portable implementation is attractive, but it prevents direct use of important CUDA libraries and backend-specific kernels. Portability at the compute API must not silently cost NVIDIA performance.

`wgpu` may be evaluated later as an additional fallback. It is not the primary CUDA or Metal execution path.

## Deferred: final training framework

Training needs are not yet known. Selecting Candle, Burn, or another framework now would guess at model operations, optimizer state, gradient communication, and backend parity.

## Consequences

- Platform code uses Cargo features and target-specific compilation.
- Networking can build and run without either GPU feature.
- Hardware capability messages use common types.
- Backend-specific optimization does not change mesh networking.
- Some models or operations may work on CUDA before Metal. Capability reporting must make this explicit.
