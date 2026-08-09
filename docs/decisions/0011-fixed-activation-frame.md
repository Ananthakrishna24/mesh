# ADR-0011: Fixed Activation Tensor Frame

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-08-09 |
| Owners | Architecture discussion |

## Context

Layer-pipeline inference sends large activation tensors between adjacent PCs. The receiver must validate identity, shape, size, and placement before allocating memory. The path should not copy a large tensor through a general control-message object.

## Decision

Use one unidirectional QUIC stream per activation. Send a fixed 128-byte binary header followed by exactly one contiguous raw tensor payload and stream finish.

The first format supports rank 1 through 4 and accepts only contiguous little-endian FP16 payloads. Qwen3 stage activations use `[batch, sequence, hidden]`. Limit one payload to 256 MiB and two active activation streams per stage and request.

Use big-endian integers in the header. Include deployment ID, request ID, transfer ID, adjacent stage indexes, prefill/decode kind, data type, dimensions, sequence position, payload length, and element count. Keep reserved bytes zero.

Canonical contract: [Activation tensor frame](../architecture/protocol/activation-frame.md)

## Rejected: activation bytes inside Protobuf

A Protobuf `bytes` field can require an additional large allocation and copy. Control serialization should not own the tensor hot path.

## Rejected: self-describing tensor formats per token

A general tensor container repeats names and metadata already fixed by the deployment. The fixed header validates the required invariants with less parsing and bounded work.

## Rejected: compression initially

FP16 activation compression may reduce WAN bytes but adds latency, backend work, negotiation, and numerical changes. It must be justified by measurements after the baseline works.

## Rejected: checksum initially

QUIC already detects transport corruption. Model and request identity checks protect semantic routing. Another payload checksum adds a full memory pass without proving a first-version benefit.

## Consequences

- The sender may copy once into a contiguous little-endian FP16 buffer when the backend cannot expose one directly.
- The receiver can reject invalid streams before allocating the declared payload.
- New layouts, ranks above four, or incompatible metadata require a new negotiated format.
- BF16 or quantized activations are later minor-version additions only after every selected backend advertises support.
- The stage queue and QUIC flow control provide backpressure without one success acknowledgment per token.
