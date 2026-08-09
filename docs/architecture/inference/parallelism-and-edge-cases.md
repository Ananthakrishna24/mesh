# Inference Parallelism and Edge Cases

| Field | Value |
|---|---|
| Status | Accepted design; advanced modes deferred |
| Canonical for | Safe parallel work and inference failure rules |
| Parent | [Distributed LLM inference](README.md) |

## Hard dependency

A normal autoregressive model cannot finish token 2 before token 1 is known. A dense transformer layer cannot run before the previous layer produces its output.

Parallel work must happen around this dependency or across independent requests.

## Supported parallel work

| Method | Helps one response | Helps total throughput | Initial support |
|---|---:|---:|---|
| Independent model replicas | No | High | Yes |
| Dynamic request batching | Sometimes | High | Yes |
| Several requests in a layer pipeline | No | High | Yes |
| Parallel model downloads | Startup only | Startup only | Yes |
| Parallel loading and warm-up | Startup only | Startup only | Yes |
| Local multi-GPU execution | Possible | High | After single-GPU path |
| Replicated slow pipeline stage | No | High | Later |
| Speculative decoding | Yes | Possible | Later |
| Remote tensor parallelism | Usually negative | Usually negative | No |

## Independent replicas

When a full model fits on several nodes, give each node different requests. This avoids per-token GPU communication and is the preferred WAN parallel mode.

## Dynamic request batching

The Request Scheduler may combine compatible requests for a short time window. Requests must use the same deployment, backend-compatible tensor shape, and generation step class.

The scheduler limits the batch using reserved KV-cache memory. It must not create a batch that can fit now but will exceed memory at the allowed context length.

## Pipeline concurrency

A layer pipeline should carry several independent sequences so different stages remain busy.

```text
             Step 1      Step 2      Step 3      Step 4
Stage A      Request 1   Request 2   Request 3   Request 4
Stage B      waiting     Request 1   Request 2   Request 3
Stage C      waiting     waiting     Request 1   Request 2
```

This improves total throughput. It does not remove the complete route delay from one sequence.

## Prompt chunks

Prefill may move prompt chunks through the stage pipeline. Later chunks still depend on the earlier KV state. The worker preserves causal order for every request.

For a hidden size of 8,192, an FP16 activation is about 16 KiB per decode token and about 64 MiB for a 4,096-token prefill boundary. Decode is usually delay-sensitive. Prefill is often bandwidth-sensitive.

## Local multi-GPU work

Communication inside one PC is normally faster than internet communication. A later local backend may use tensor or pipeline parallelism across GPUs in the same PC.

The mesh sees that local GPU group as one stage endpoint. WAN planning remains continuous-layer pipeline planning.

## Speculative decoding

A small draft model may propose several tokens. The target model validates them together. This can reduce complete target-model passes per accepted token.

Prefer running the draft model on a pipeline endpoint instead of adding another WAN round trip. This mode requires a compatible draft model and real quality and speed measurements.

## Rejected initial method: remote tensor parallelism

Remote tensor parallelism exchanges partial results several times inside many layers. Normal internet delay makes this a poor default. It may be enabled only for a measured low-delay, high-bandwidth link that beats the local or layer-pipeline alternative.

## Required edge-case rules

### Competing coordinators

Two coordinators may choose the same GPU from stale capability reports. The Local Resource Manager is authoritative. Every deployment requires an expiring local reservation.

### Growing KV cache

Memory planning uses maximum accepted context length, concurrent sequences, layer assignment, and KV data type. A deployment must reject a request whose declared limits exceed its reservation.

### Apple unified memory

Metal shares memory with the operating system. Report a safe available limit, not total system memory. Recheck the limit before accepting a reservation.

### Backend differences

CUDA and Metal may support different operations, data types, and quantization kernels. A node advertises tested runtime capabilities. GPU memory alone does not make a node compatible.

### Unequal layers

Placement uses measured layer-group memory and execution time. It does not divide only by layer count. Mixture-of-experts layers need separate measurements.

### Shared embedding and output weights

Some models tie the input embedding and output head. If they are placed on different nodes, the plan accounts for duplicated weights or explicitly places both functions together.

### Stage loss

A disconnected stage owns part of every active KV cache. The first version stops those requests and restarts them from the prompt after replanning. It does not migrate live KV state.

### Placement lifetime

Layer placement remains fixed while requests use the deployment. Rebalance only after draining or cancelling active requests.

### Slow stage and backpressure

Every stage has a bounded input queue. A stage advertises when it can accept more work. Earlier stages stop producing when the next queue is full.

### Model mismatch

Every stage reports the same immutable model revision, manifest hash, quantization, tokenizer identity, and placement-plan ID before commit. A mismatch aborts preparation.

### Numerical differences

CUDA and Metal may produce small floating-point differences. The final stage owns sampling state and random seed. Tests use tolerances for tensors and exact checks only where the contract guarantees exactness.

### Cancellation

Cancellation propagates to every stage. Workers remove queued activations, KV cache, temporary model references, and resource reservations for that request or deployment.

### Download interference

Active inference control and activation transfers have priority over model downloads and background peer cache sharing. Model downloads are rate-limited while inference is active.

### Partial connection graph

The planner uses the measured connection graph. Each adjacent stage must have a direct link. The coordinator must have a direct control link to every selected peer. Total GPU memory is irrelevant when the required direct links do not exist.

### Partial prepare failure

The coordinator commits only after every required stage reports `READY`. One rejection, hash mismatch, load failure, or timeout releases the full pending plan.

### Provider failure

A valid partial download remains in the cache. An incomplete artifact is never marked ready. Preparation may retry another provider source or peer before the reservation expires.

## Build order

1. Complete model on one node.
2. Independent requests on full-model replicas.
3. Dynamic request batching.
4. Continuous-layer pipeline.
5. Several concurrent sequences in the pipeline.
6. Parallel provider downloads, loading, and warm-up.
7. Multiple local GPUs inside one PC.
8. Replicated bottleneck stages.
9. Speculative decoding.
10. Experimental methods only after measurements.
