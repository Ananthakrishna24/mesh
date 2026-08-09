# Distributed LLM Inference

| Field | Value |
|---|---|
| Status | Accepted design; not implemented |
| Canonical for | Inference modules, placement, reservation, and execution |
| Parent | [Architecture overview](../README.md) |
| Related | [Parallelism and edge cases](parallelism-and-edge-cases.md) |
| Related | [Provider-backed model distribution](model-distribution.md) |
| First model family | [Qwen3 dense 4B and 8B](qwen3-model-family.md) |

## Goal

Run LLM inference on one or more internet-connected PCs. Use more than one PC when it increases request throughput or when the model cannot fit on one PC.

The system must not claim that several remote GPUs become one local GPU. The Inference Coordinator creates a placement plan over independent resources.

## First model proofs

- `Qwen/Qwen3-4B` proves complete-model inference on Windows CUDA, Linux CUDA, and macOS Metal.
- `Qwen/Qwen3-8B` proves provider-driven partial weights and a continuous-layer pipeline across at least two PCs.

Both use the same dense Qwen3 Model Family Adapter. The first correctness profile uses unquantized Safetensors, FP16 runtime weights and wire activations, a 4,096-token limit, batch size 1, and non-thinking mode.

Canonical contract: [Qwen3 dense model family](qwen3-model-family.md)

## Terms

- **Model stage:** one continuous range of layers from the same model.
- **Placement plan:** the exact nodes, layer ranges, route, memory reservations, and model revision for one inference deployment.
- **Inference deployment:** a prepared model placement that may serve one or more requests.
- **Inference request:** one prompt and its generated output.
- **Replica:** one complete copy of the model on a node or local GPU group.

Do not call layer ranges submodels. They cannot run independently.

## New modules

```text
Existing mesh data
├── Peer Store
├── Hardware Scanner
├── Direct Link Manager
└── Network Profiler
           │
           ▼
Job owner
└── Inference Coordinator
    ├── Model Resolver
    ├── Placement Planner
    └── Request Scheduler
           │
           │ placement and reservation messages
           ▼
Every participating peer
├── Local Resource Manager
├── Model Store
└── Inference Worker
    ├── CUDA backend
    └── Metal backend
```

### Inference Coordinator

A temporary module created by the job owner. It controls one inference deployment, not the mesh.

It:

1. Resolves an exact model revision.
2. Reads node capabilities and measured link performance.
3. Chooses an inference mode.
4. Creates the placement plan.
5. Reserves resources on selected nodes.
6. Waits for model stages to download, load, and warm up.
7. Commits the deployment only when every stage is ready.
8. Schedules requests and returns output.

### Model Resolver

Converts a provider model reference into one immutable model identity and a normalized model manifest.

Canonical flow: [Provider-backed model distribution](model-distribution.md)

### Placement Planner

Creates a plan from:

- Exact model revision and format.
- Layer and global-weight memory.
- KV-cache memory for the allowed context and batch size.
- Supported CUDA or Metal operations and data types.
- Available GPU and system memory.
- Measured compute speed.
- Measured delay and bandwidth for each direct peer link.
- Model artifacts already cached by each node.

The output assigns continuous layer ranges. It uses the smallest practical number of nodes.

### Request Scheduler

Owns active request queues, local batching, cancellation, generated-token routing, and deployment-level concurrency limits.

### Local Resource Manager

Each peer is authoritative for its own resources. It:

- Reports current capacity.
- Offers resources for a proposed plan.
- Creates expiring reservations.
- Accepts or rejects the final commit.
- Prevents two coordinators from using the same memory.
- Releases resources after completion, cancellation, failure, or lease expiry.

The Placement Planner proposes. The Local Resource Manager decides locally.

### Model Store

Caches immutable model artifacts on disk. It follows a placement plan and obtains only the assigned tensors when the provider format allows it.

It does not choose layer placement.

### Inference Worker

Loads one assigned model stage, owns that stage's KV cache, runs CUDA or Metal operations, and sends activations to the next stage.

## Inference modes

The planner checks modes in this order.

### Mode 1 — Single node

Use one node when the complete model and required KV cache fit.

This normally gives the lowest delay.

### Mode 2 — Full-model replicas

Place a complete model copy on several nodes. Send different requests to different replicas.

This is the preferred way to increase throughput over the internet.

### Mode 3 — Continuous-layer pipeline

Split a model across the smallest group of nodes that can hold it.

```text
Token ID
   │
   ▼
PC A: embedding + layers 0–11
   │ activation
   ▼
PC B: layers 12–27
   │ activation
   ▼
PC C: layers 28–39 + output head
   │ next token ID
   └──────────────────────────▶ PC A
```

The last stage performs sampling. It sends the next token ID to the first stage and the generated token to the coordinator.

Remote tensor parallelism is not an initial mode. It synchronizes several times inside each layer and is unsuitable for ordinary internet links.

## Placement algorithm

1. Resolve the provider reference to an immutable model revision.
2. Read the normalized model manifest.
3. Calculate weight memory, duplicated global weights, runtime work memory, and KV-cache reserve.
4. Exclude nodes that cannot run the required operations, data type, or quantization.
5. Exclude links that are unavailable or below the deployment's minimum requirements.
6. Check whether one node can hold the complete deployment.
7. Otherwise, check whether full replicas fit on several independent nodes.
8. Otherwise, find the smallest connected node group that can hold one layer pipeline.
9. Order pipeline nodes using measured directional delay and bandwidth.
10. Assign continuous layer ranges using measured layer time and memory, not layer count alone.
11. Ask every selected node for a resource offer.
12. Reserve every required node with an expiry time.
13. Send the model preparation plan.
14. Wait for every stage to report the same deployment identity and `READY` state.
15. Run a warm-up request.
16. Commit the deployment and accept inference requests.
17. Release every reservation if any required stage fails before commit.

## Resource reservation protocol

```text
Coordinator                       Selected peers
    │                                  │
    ├── RESOURCE_QUERY ───────────────▶ │
    │ ◀────────────── RESOURCE_OFFER ──┤
    ├── RESERVE_REQUEST ──────────────▶ │
    │ ◀────────── ACCEPTED / REJECTED ─┤
    ├── PREPARE_MODEL ────────────────▶ │
    │ ◀────────────────────── READY ───┤
    ├── RESERVATION_COMMIT ───────────▶ │
    │                                  │
    └── RELEASE when finished ────────▶ │
```

If one node rejects or times out, the coordinator releases every accepted reservation and creates a new plan. Reservations expire if the coordinator disconnects.

## Pipeline request flow

1. The coordinator creates a unique request ID.
2. The first stage tokenizes or receives token IDs and creates embeddings.
3. Each stage receives an activation, runs its continuous layer range, and sends one activation to the next stage.
4. Each stage keeps the KV cache for its own layers.
5. The final stage calculates token scores and samples the next token.
6. The final stage sends the token ID to the first stage.
7. The final stage also sends the token result to the coordinator.
8. The loop stops on end-of-sequence, length limit, cancellation, or failure.

Do not send KV caches across nodes during normal generation.

## Wire tensor rule

CUDA and Metal device objects never cross the network.

A worker copies an activation into a negotiated wire representation. The first representation should be FP16 when every selected backend supports the required operations.

Each transfer identifies:

- Deployment ID.
- Request ID.
- Token or prompt-chunk position.
- Source and destination stage.
- Tensor shape.
- Data type.
- Payload length.

Activation compression and INT8 transfer remain measurement-driven optimizations.

## Failure policy for the first version

If a stage disconnects during generation, stop the affected requests and release the deployment after its lease expires or the coordinator performs cleanup.

Do not move a live stage in the first version. Its KV cache is not available on another node. Recreate the placement and restart from the prompt.

## Capacity statement

Yes, layer placement can run a model whose weights and KV cache do not fit on one PC. The usable capacity is less than the sum of all GPU memory because each node needs runtime work memory, KV cache, and sometimes duplicated embedding or output weights.

A larger model may run, but internet delay can make it slower. Capacity and speed are separate goals.
