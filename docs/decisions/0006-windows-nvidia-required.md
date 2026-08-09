# ADR-0006: Windows NVIDIA CUDA Is a Required Target

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-08-09 |
| Owners | Architecture discussion |

## Context

The mesh must enroll and use NVIDIA GPUs on Windows as first-class compute peers. Treating Windows as a later platform would allow networking, GUI, model, and CUDA assumptions to become Linux-specific.

## Decision

The required first-class host targets are:

| Rust target | GPU backend | Requirement |
|---|---|---|
| `x86_64-pc-windows-msvc` | NVIDIA CUDA | Required |
| `x86_64-unknown-linux-gnu` | NVIDIA CUDA | Required |
| `aarch64-apple-darwin` | Apple Metal | Required |

All three targets use the same mesh, enrollment, model-provider, placement, and inference protocols. A deployment may create a pipeline containing Windows CUDA, Linux CUDA, and macOS Metal peers when the selected model and wire data type are supported by every stage.

## Windows product contract

- The native eframe GUI is required.
- GUI-driven create and enroll flows are required.
- Quinn direct connections are required.
- NVML hardware discovery is required for supported NVIDIA drivers.
- CUDA inference is required.
- Hugging Face model access and Safetensors caching are required.
- Windows Firewall failures must have a guided recovery screen.
- Provider credentials use a Windows credential store integration when available.
- Application state uses the normal Windows application-data location.
- Packaged users open one application and do not install Rust or build tools.

## Source-build contract

`cargo run --release` remains the project command after platform prerequisites exist. A native CUDA source build may require:

- Rust's MSVC toolchain.
- Microsoft C++ Build Tools.
- A compatible NVIDIA driver.
- CUDA development tools required by the selected inference backend.

These are platform prerequisites, not extra project commands. Packaged releases must not require normal users to compile CUDA code.

## Candle validation rule

Candle remains the proposed first inference engine. Native Windows CUDA support must be proven with the selected model family, quantization, kernels, and MSVC toolchain before the inference phase is accepted.

If Candle cannot satisfy the Windows CUDA proof, replace or specialize the Windows CUDA implementation behind the existing compute-backend boundary. Do not remove Windows from the target matrix.

## Required platform proofs

Before a phase is complete, verify its relevant behavior on Windows:

1. Build and launch the native application.
2. Persist and reload node identity.
3. Create and open `.mesh-invite` files.
4. Complete direct QUIC enrollment with a Linux or macOS peer.
5. Explain Windows Firewall blocks through the GUI.
6. Detect NVIDIA GPU, memory, driver, and CUDA capability.
7. Resolve and cache an immutable provider model.
8. Run the selected single-node inference model through CUDA.
9. Exchange activation tensors with another supported platform.
10. Restart and recover peer, cache, and deployment state.

## Rejected: WSL-only Windows support

WSL may help developers, but it is not Windows product support. The required application is a native Windows executable using the Windows GUI, networking, storage, credentials, and NVIDIA runtime.

## Rejected: networking-only Windows peer

A Windows PC with a supported NVIDIA GPU must be usable as an inference worker. Networking-only operation is an honest fallback when CUDA initialization fails, not the completed Windows target.

## Consequences

- Native Windows builds and manual proofs are required from the first relevant phase. Windows CI begins after the first confident Qwen3-4B CUDA implementation, before replica and distributed inference work proceeds.
- Platform-specific modules stay behind Rust traits and Cargo target gates.
- Windows firewall, DLL discovery, credentials, and installer behavior become planned work.
- CUDA and model-family choices must be validated on both Windows and Linux.
- AMD GPUs remain outside the first target matrix.

## Sources

- [NVIDIA CUDA Installation Guide for Microsoft Windows](https://docs.nvidia.com/cuda/cuda-installation-guide-microsoft-windows/)
- [egui and eframe](https://github.com/emilk/egui)
- [Quinn](https://github.com/quinn-rs/quinn)
- [Candle](https://github.com/huggingface/candle)
