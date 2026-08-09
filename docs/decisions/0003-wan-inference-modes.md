# ADR-0003: WAN Inference Modes and Local Reservations

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-08-09 |
| Owners | Architecture discussion |

## Context

Autoregressive token generation is sequential. Internet links have much higher delay than local GPU links. The mesh still needs to increase request throughput and run models that do not fit on one PC.

Several decentralized job owners may also attempt to use the same peer at the same time.

## Decision

Support inference modes in this order:

1. One complete model on one node.
2. Full-model replicas for independent requests.
3. Continuous-layer pipeline across the smallest practical node group.

Add dynamic request batching and concurrent pipeline sequences for throughput.

Every peer runs a Local Resource Manager. A coordinator must obtain expiring reservations from every selected peer before model preparation and inference commit.

Canonical architecture: [Distributed LLM inference](../architecture/inference/README.md)

## Rejected initial mode: remote tensor parallelism

Tensor parallelism exchanges partial results several times inside many layers. It is appropriate for fast local interconnects, not an ordinary WAN path. A future measured link may opt in, but it is not a general mesh mode.

## Deferred modes

- Speculative decoding.
- Replicated bottleneck stages.
- Live KV-cache replication.
- Live stage migration.
- Remote mixture-of-experts routing.

## Consequences

- One response may be slower when split across PCs.
- Full-model replicas are the preferred WAN throughput method.
- Layer pipeline increases model capacity.
- A disconnected pipeline stage stops requests that depend on its KV cache.
- Placement uses measured compute, memory, delay, and bandwidth.
- A peer may reject a stale placement proposal even when the coordinator expected capacity.
