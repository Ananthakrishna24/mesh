# GPU Backends

| Field | Value |
|---|---|
| Status | Platform support accepted; execution libraries proposed |
| Canonical for | NVIDIA and Apple GPU boundaries |
| Parent | [Architecture overview](../README.md) |

## Accepted targets

The first node version supports:

1. NVIDIA GPUs through CUDA.
2. Apple GPUs through Metal on macOS.

These are separate native paths behind one Rust interface.

## Why not one generic GPU API?

A generic API such as `wgpu` can run on several platforms. It is useful for portable graphics and some compute. It does not expose the complete CUDA library ecosystem used by high-performance NVIDIA machine learning.

Using only one generic layer would make the code look simpler while limiting backend-specific performance. The architecture keeps a common Rust interface and allows each backend to use its native fast path.

## Separate discovery from execution

Hardware discovery must work before LLM inference or training exists.

```text
Hardware Scanner
    ├── CUDA Probe  ──▶ NVIDIA device and memory report
    └── Metal Probe ──▶ Apple device and memory report

GPU Worker
    └── Compute Backend
          ├── CUDA Backend
          └── Metal Backend
```

The Hardware Scanner reports capability. The GPU Worker executes work. Do not make hardware discovery depend on loading a model framework.

## Proposed Rust boundary

```rust
pub trait HardwareProbe {
    fn discover(&self) -> Result<Vec<GpuDeviceInfo>, HardwareError>;
}

pub struct GpuDeviceInfo {
    pub backend: GpuBackendKind,
    pub stable_id: String,
    pub name: String,
    pub total_memory_bytes: u64,
    pub available_memory_bytes: Option<u64>,
    pub runtime_version: Option<String>,
}

pub enum GpuBackendKind {
    Cuda,
    Metal,
}
```

The execution trait will be defined with the first real inference or training workload. Defining it now would guess at model, tensor, and synchronization requirements.

## NVIDIA CUDA path

### Discovery

**Proposed library:** `nvml-wrapper`.

NVML is NVIDIA's management library. It can report GPU identity, memory, utilization, temperature, and driver information without loading a machine-learning model.

The node must treat NVML as optional at runtime. A missing driver should produce an unsupported-device report, not crash the node.

### Compute

**Proposed first framework:** Hugging Face Candle with its `cuda` feature.

Candle provides Rust tensors and a CUDA device through the same model API used by its Metal backend. This reduces initial model integration work.

For later performance work, CUDA-specific kernels or libraries may be used behind the CUDA backend. The common interface must not prevent that.

## Apple Metal path

### Discovery

**Proposed library:** `objc2-metal` or the maintained Metal bindings selected during implementation.

The probe should use the native Metal device API to report device name, available capabilities, and memory information exposed by macOS.

Metal is available only on Apple platforms. The Metal crate and code must be behind a Cargo feature and target-specific compilation.

### Compute

**Proposed first framework:** Hugging Face Candle with its `metal` feature.

Candle documents separate `cuda` and `metal` features and can construct tensors on those devices. This makes it a strong first inference candidate across the two accepted GPU targets.

## Is Candle the final training choice?

No decision yet.

Candle is a good first inference candidate because it supports CUDA and Metal from Rust. Training requirements are broader. Distributed optimization, missing operations, backend parity, and model-specific kernels must be measured with a real training workload.

The training framework remains **Deferred**. Candidates can include Candle, Burn, or backend-specific code when the training architecture is discussed.

## Feature shape

The intended Cargo feature boundary is:

```toml
[features]
default = []
cuda = []
metal = []
```

Rules:

- Linux and Windows NVIDIA builds may enable `cuda`.
- macOS Apple Silicon builds may enable `metal`.
- A build may contain neither backend and still run node networking.
- Unsupported backend code must not compile on the wrong platform.
- Peer capability messages report only backends that initialized successfully.

Exact dependencies and versions belong in the Rust workspace once implementation begins.

## Efficiency decision

- Use CUDA-native execution for NVIDIA.
- Use Metal-native execution for Apple GPUs.
- Use Candle first when it supports the required model and operations.
- Measure real workloads before replacing a working Candle path.
- Allow optimized backend-specific kernels without changing mesh networking.

This is more efficient and maintainable than forcing both vendors through one lowest-common-denominator backend.

## Sources

- [Candle repository](https://github.com/huggingface/candle)
- [Candle CUDA and Metal installation features](https://github.com/huggingface/candle/blob/main/candle-book/src/guide/installation.md)
- [NVIDIA Management Library](https://developer.nvidia.com/management-library-nvml)
- [Apple Metal documentation](https://developer.apple.com/metal/)
