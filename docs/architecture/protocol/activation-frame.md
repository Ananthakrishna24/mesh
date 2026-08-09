# Activation Tensor Frame

| Field | Value |
|---|---|
| Status | Accepted |
| Canonical for | Distributed inference activation header, payload, validation, and limits |
| Parent | [Distributed LLM inference](../inference/README.md) |
| Decision | [ADR-0011: Fixed activation frame](../../decisions/0011-fixed-activation-frame.md) |

## Boundary

An activation is the tensor output of one model stage and the input of the next stage. It is the large value that crosses a direct peer connection during layer-pipeline inference.

The first wire format uses one unidirectional QUIC stream for one activation. The stream contains one fixed 128-byte header, one raw tensor payload, and a normal stream finish.

```text
QUIC unidirectional stream
+---------------------------+----------------------------------+-----+
| 128-byte ActivationHeader | payload_len raw tensor bytes     | FIN |
+---------------------------+----------------------------------+-----+
```

Control messages and model files do not use this frame.

## Header layout

All multi-byte header integers use big-endian network byte order.

| Offset | Size | Field | First-version rule |
|---:|---:|---|---|
| 0 | 4 | Magic | ASCII `MSHA` |
| 4 | 2 | Wire major | `1` |
| 6 | 2 | Wire minor | `0` |
| 8 | 16 | Deployment ID | Exact accepted deployment |
| 24 | 16 | Request ID | Exact active request |
| 40 | 8 | Transfer ID | Monotonically increasing within the request |
| 48 | 2 | Source stage | Accepted zero-based stage index |
| 50 | 2 | Destination stage | Must equal source stage plus one |
| 52 | 1 | Transfer kind | `1` prefill, `2` decode |
| 53 | 1 | Data type | `1` IEEE 754 FP16 |
| 54 | 1 | Rank | `1..4`; Qwen3 activation rank is `3` |
| 55 | 1 | Flags | Must be zero |
| 56 | 32 | Dimensions | Four `u64` values; unused values are zero |
| 88 | 8 | Sequence position | First token position represented by this activation |
| 96 | 8 | Payload length | Raw payload bytes following the header |
| 104 | 8 | Element count | Product of used dimensions |
| 112 | 16 | Reserved | Must be all zero |

IDs are raw 16-byte values. They are not text UUIDs on the wire.

## Tensor layout

The first Qwen3 stage activation is:

```text
shape  = [batch, sequence, hidden]
layout = contiguous row-major
value  = IEEE 754 binary16
bytes  = little-endian FP16 values
```

Rules:

- `element_count` equals the checked product of the `rank` used dimensions.
- `payload_len` equals `element_count * 2` for FP16.
- Zero-sized dimensions are invalid.
- Unused dimensions must be zero.
- The payload has no padding, compression, or embedded metadata.
- A stream with fewer or more than `payload_len` bytes is invalid.
- QUIC supplies transport integrity. The first format adds no checksum.

Header byte order and tensor byte order are intentionally different: headers use network byte order; tensor elements use little-endian order used by the accepted hosts and GPU transfer path.

## Limits

The first protocol enforces:

- Maximum header rank: 4.
- Maximum payload: 268,435,456 bytes, or 256 MiB.
- Maximum active activation streams per stage and request: 2.
- Maximum stage index: 65,535 by representation; a deployment declares a much smaller exact stage count.
- No activation compression.
- No multipart activation. QUIC may packetize the stream internally, but the application sees one logical tensor.

A deployment may advertise a smaller payload or in-flight limit. It may not exceed the protocol maximum.

## Sender algorithm

1. Obtain the next transfer ID for the request.
2. Confirm the destination stage and peer match the immutable placement plan.
3. Confirm the tensor is contiguous FP16 with the expected shape.
4. Check the payload limit.
5. Open one unidirectional QUIC stream.
6. Write the complete 128-byte header.
7. Write exactly `payload_len` bytes.
8. Finish the stream.
9. Keep at most the accepted number of activation streams in flight.

The sender does not wait for a success control message after every tensor. QUIC flow control and the bounded receiver queue provide normal backpressure. A validation or stage failure uses a control error and request cancellation.

## Receiver algorithm

Validate before allocating the final tensor buffer or copying to a GPU:

1. Read exactly 128 bytes.
2. Check magic and wire major.
3. Check every reserved byte and flag.
4. Match deployment ID and request ID to an active accepted request.
5. Match source, destination, transfer kind, and sequence position to expected request state.
6. Reject a duplicate transfer ID. Independent QUIC streams may finish out of order, so a lower transfer ID is not invalid by itself.
7. Validate rank, dimensions, element count, data type, and payload length with checked arithmetic.
8. Confirm payload and queue limits.
9. Confirm the Local Resource Manager still owns the required memory reservation.
10. Allocate one bounded host receive buffer or approved direct-transfer buffer.
11. Read exactly the declared payload and require stream finish.
12. Construct the backend tensor and pass it to the assigned stage.

Never allocate from `payload_len` until all fixed limits, shape rules, placement state, and reservation state pass.

At most two validated activations may wait for one stage. The stage orders them by request state and sequence position before execution; QUIC stream arrival order does not define model order.

## Failure and cancellation

- Unknown major version: reset the stream and cancel the request with `UNSUPPORTED_PROTOCOL`.
- Unknown data type or transfer kind: reset and report `TRANSFER_REJECTED`.
- Invalid header, shape, or byte count: reset and report `MALFORMED_FRAME`.
- Unknown or stale deployment/request: reset and report `INVALID_STATE`.
- Missing memory reservation or full queue: reset and report `RESOURCE_BUSY`.
- Request cancellation resets unread activation streams and releases their buffers.
- A failed activation cancels the complete inference request. It does not silently skip a stage or retry on another placement.

## Version evolution

The 128-byte header remains fixed for wire major 1.

A later minor version may define a new data type, transfer kind, or flag only after capability negotiation. A version-1.0 receiver rejects every value it does not know. Fields cannot be resized or moved within major 1. A format needing more dimensions or metadata requires a new wire major or a separately negotiated stream format.

## Backend boundary

`mesh-net` reads and writes the wire frame. `mesh-core` owns storage-neutral header values and validation errors. `mesh-inference` verifies request and stage state. `mesh-compute` receives a validated tensor description and bytes; it never parses network headers.
