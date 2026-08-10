# ADR-0012: Network Benchmark and Placement Cost

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-08-10 |
| Owners | Architecture discussion |
| Gate | A07 |

## Context

P03 must discover hardware and measure direct peer links so later placement can choose single-node, replica, or continuous-layer pipeline modes. The Placement Planner already requires measured delay, bandwidth, stability, compute speed, and rejection thresholds. Those numbers cannot be invented from advertised internet plans or GPU marketing clocks.

## Decision

Accept the Network Profiler and placement-cost contract in [Network benchmark and placement cost](../architecture/networking/network-benchmark.md).

Key rules:

- Delay and bandwidth are directional and age out after fixed fresh, stale, and expired windows.
- Delay uses multiple control-stream RTT samples; planning one-way delay is half the retained mean RTT.
- Bandwidth uses one unidirectional QUIC stream with a fixed 32-byte header and at most 16 MiB payload.
- Stability is derived from the newest delay measurement only.
- Continuous-layer pipeline hops hard-reject above 80 ms one-way delay, below 10 Mbps directional bandwidth, or stability under 50.
- Maximum WAN pipeline stage count is 3.
- Local compute proxies come from Hardware Scanner; first GPU token rates come from real stage warm-up, not a synthetic pre-backend claim.
- Placement cost is estimated decode milliseconds as stage compute plus aged hop delay and activation transfer time.

## Rejected: assume symmetric or advertised link speed

Consumer links are often asymmetric. ISP plan numbers do not describe the direct peer path, NAT path, or Wi-Fi hop actually used by Quinn.

## Rejected: true one-way latency with wall clocks

Peer clocks are not synchronized. Half-RTT is an explicit planning estimate and is good enough for mode selection thresholds.

## Rejected: continuous high-rate probing

Always-on large transfers interfere with enrollment, control, and later activations. Benchmarks are bounded, concurrent-limited, and yield to inference.

## Rejected: unlimited pipeline depth over WAN

Each added stage adds at least one more internet hop and activation transfer. Three stages is enough for the Qwen3-8B proof without inviting long high-delay chains.

## Consequences

- P03 can implement capability reports and directional benchmarks against a locked contract.
- `mesh-core` gains storage-neutral hardware and link measurement types.
- `mesh-net` gains benchmark stream handling separate from the control stream.
- `mesh-inference` placement must call the published thresholds and cost formula instead of embedding ad hoc constants.
- GUI dashboards must show measured, stale, expired, or unavailable labels rather than placeholder speeds.
