# Network Benchmark and Placement Cost

| Field | Value |
|---|---|
| Status | Accepted |
| Canonical for | Directional link measurement, measurement age, compute probes, pipeline cost, and placement rejection thresholds |
| Parent | [Distributed LLM inference](../inference/README.md) |
| Decision | [ADR-0012: Network benchmark and placement cost](../../decisions/0012-network-benchmark-and-placement-cost.md) |
| Related | [Direct connection algorithm](direct-connection.md) |
| Related | [Node modules](../system/node-modules.md) |
| Implements gate | A07 |

## Boundary

The Network Profiler owns measured direct-link performance. The Hardware Scanner owns local device discovery and local compute probes. The Placement Planner consumes both. No module invents internet speed or GPU speed from marketing numbers.

Measurements are directional. `A → B` is not assumed equal to `B → A`.

Large benchmark payloads use a dedicated unidirectional QUIC stream. They never ride the control stream.

## Terms

- **Link sample:** one completed delay or bandwidth observation in one direction.
- **Link measurement:** the aggregated, time-stamped summary the planner reads.
- **Measurement age:** wall-clock time since `measured_at_unix_ms`.
- **Stability score:** integer `0..=100` summarizing recent delay variance and probe success.
- **Pipeline hop:** the direct path between two adjacent stages.
- **Placement cost:** estimated milliseconds per decode token for one continuous-layer route.

## Directional delay

### Method

1. After a successful mesh handshake, either peer may start a delay probe on the control stream.
2. The initiator sends `BenchmarkRequest` with `kind = DELAY` and a monotonic probe id.
3. The responder replies on the control stream with `BenchmarkAccept` or `BenchmarkReject`.
4. The initiator sends `N` `Heartbeat` frames that carry `sent_at_unix_ms`.
5. The responder immediately replies to each heartbeat with `in_reply_to` set and its own `sent_at_unix_ms`.
6. The initiator records RTT as local receive time minus local send time using a monotonic clock.
7. One-way delay for planning is `rtt_ms / 2.0`. The system does not claim true one-way latency without synchronized clocks.

### Delay parameters

| Parameter | Value |
|---|---|
| Probe count per measurement | 7 |
| Discarded extremes | highest and lowest RTT |
| Inter-probe spacing | 20 ms |
| Per-probe timeout | 1,000 ms |
| Full delay measurement deadline | 5 s |
| Concurrent delay measurements per peer | 1 |

### Delay summary fields

| Field | Rule |
|---|---|
| `rtt_ms` | Mean of the five retained RTT samples |
| `one_way_delay_ms` | `rtt_ms / 2.0` |
| `rtt_p95_ms` | 95th percentile of retained samples, or max when fewer than five remain after discard |
| `sample_count` | Number of successful RTT samples before discard |
| `loss_count` | Timed-out or failed probes |

If fewer than three successful samples remain, the measurement is invalid and must be retried or marked failed.

## Directional bandwidth

### Method

1. The initiator sends `BenchmarkRequest` with `kind = BANDWIDTH`, desired payload bytes, and direction.
2. The responder accepts only when it can allocate the receive buffer and is not already at the benchmark concurrency limit.
3. The sender opens one unidirectional QUIC stream.
4. The stream begins with a fixed 32-byte `BenchmarkStreamHeader`, then exactly `payload_len` zero-filled or pseudo-random bytes, then FIN.
5. The receiver validates the header before accepting the payload and measures wall time from first payload byte to stream finish.
6. Both peers exchange `BenchmarkResult` on the control stream.

Direction names are always from the reporter's view:

- `TO_PEER`: this node sent the payload.
- `FROM_PEER`: this node received the payload.

A full duplex characterization runs both directions, not at the same time on the same link.

### Benchmark stream header

```text
Offset  Size  Field
0       4     Magic ASCII `MSHB`
4       2     Header version = 1
6       2     Reserved = 0
8       16    Probe ID
24      8     Payload length (u64 BE)
```

Rules:

- Maximum payload: 16 MiB.
- Default payload: 4 MiB.
- Minimum accepted payload for a valid bandwidth number: 256 KiB.
- Maximum active benchmark streams per node: 1.
- Benchmark streams yield to activation and control traffic. Active inference rejects new bandwidth probes with `RESOURCE_BUSY`.
- Bandwidth is `payload_bytes * 8 / elapsed_seconds` bits per second.
- Elapsed time uses a monotonic clock and excludes header-only setup before the first payload byte when the receiver can observe that boundary. If it cannot, elapsed time starts at stream accept and the result is still accepted with that definition recorded locally.

### Bandwidth summary fields

| Field | Rule |
|---|---|
| `bandwidth_bps` | Measured bits per second |
| `payload_bytes` | Accepted payload length |
| `transfer_ms` | Elapsed milliseconds |
| `measured_at_unix_ms` | UTC Unix ms when the result was finalized |

## Stability

After each delay measurement:

```text
stability_score =
    clamp_0_100(
        100
        - 4 * loss_count
        - min(40, round(200 * stddev_rtt_ms / max(rtt_ms, 1.0)))
    )
```

Interpretation for planners:

| Score | Meaning |
|---:|---|
| 80..=100 | Stable enough for pipeline placement |
| 50..=79 | Usable for replicas; pipeline only if no better route exists |
| 0..=49 | Reject for pipeline hops; prefer single-node or replica modes |

Stability is recomputed from the newest delay measurement. Older stability values are not blended across expired samples.

## Measurement age

| Age | State | Planner rule |
|---|---|---|
| `< 5 minutes` | Fresh | Use directly |
| `5 minutes .. 30 minutes` | Stale | Usable with a 1.25× delay multiplier and 0.8× bandwidth multiplier |
| `> 30 minutes` | Expired | Do not use for a new placement; remeasure first |
| Missing | Unknown | Treat the link as unmeasured |

A connected peer pair should refresh delay at least every 10 minutes while both are idle enough to accept a probe. Bandwidth refreshes every 30 minutes, after reconnect, or when the previous bandwidth measurement is expired.

Remeasurement is skipped while an inference deployment is actively transferring activations on that link, except for lightweight delay probes that fit the control budget.

## Compute benchmarks

Local compute probes belong to the Hardware Scanner and are independent of peer networking.

### First probe set

| Probe | What it measures | Output |
|---|---|---|
| CPU identity | Model and logical cores | Report fields only |
| Memory / disk | Capacity and currently available bytes | Report fields only |
| GPU identity | Backend, name, memory, driver or runtime | One record per usable device |
| GPU availability | NVML or Metal probe success | Device omitted when probe fails |
| Compute speed proxy | Sustained host FP32 multiply-add throughput on one worker thread for 200 ms | `cpu_fp32_gflops` |
| Optional GPU proxy | Reserved for the first inference backend warm-up sample | Stored only after a real backend run |

The first placement-critical GPU speed number comes from measured stage warm-up during deployment preparation, not from a synthetic kernel invented before the compute backend exists. Until that sample exists, the planner may use memory fit and backend compatibility only, and must not invent token-per-second claims.

### Compute measurement age

Use the same fresh, stale, and expired windows as network measurements. An expired compute proxy must be refreshed before it influences ranking among otherwise equal plans. Memory fit checks always re-read current available memory at reservation time.

## Capability report

Every node builds a local capability report:

```text
CapabilityReport
├── collected_at_unix_ms
├── os, arch
├── cpu_model, cpu_logical_cores
├── memory_total_bytes, memory_available_bytes
├── disk_total_bytes, disk_available_bytes
├── gpus[]
│   ├── backend: cuda | metal
│   ├── stable_id
│   ├── name
│   ├── total_memory_bytes
│   ├── available_memory_bytes?
│   └── runtime_version? / driver_version?
└── compute
    ├── cpu_fp32_gflops
    └── measured_at_unix_ms
```

Rules:

- Reports include only backends that initialized successfully.
- Apple Metal reports a safe available-memory estimate, not total unified memory as freely usable.
- Missing NVIDIA driver or Metal device yields an empty GPU list and a clear local status string. The node remains usable for networking.
- Peers exchange capability summaries after handshake and when the local report changes materially (GPU set change, memory availability change beyond 10%, or compute probe refresh).

## Link measurement record

```text
LinkMeasurement
├── local_node_id
├── peer_node_id
├── delay
│   ├── one_way_delay_ms
│   ├── rtt_ms
│   ├── rtt_p95_ms
│   ├── stability_score
│   ├── sample_count
│   ├── loss_count
│   └── measured_at_unix_ms
├── to_peer_bandwidth
│   ├── bandwidth_bps
│   ├── payload_bytes
│   ├── transfer_ms
│   └── measured_at_unix_ms
└── from_peer_bandwidth
    ├── bandwidth_bps
    ├── payload_bytes
    ├── transfer_ms
    └── measured_at_unix_ms
```

Each peer stores the measurements it observed. The planner on the job owner uses the measurements available at plan time and prefers the freshest matching direction.

## Pipeline cost model

For one decode token on a continuous-layer route:

```text
activation_bytes = batch * sequence * hidden * bytes_per_element
transfer_ms(link) = (activation_bytes * 8 * 1000) / bandwidth_bps
hop_ms(link) = aged_one_way_delay_ms(link) + transfer_ms(link)

placement_cost_ms =
    sum(stage_compute_ms[stage])
  + sum(hop_ms[link] for each adjacent stage pair)
  + final_sample_ms
```

First-profile constants:

- `batch = 1`
- decode `sequence = 1`
- `bytes_per_element = 2` (FP16)
- Qwen3-4B hidden size comes from the resolved manifest; approximate planning may use 2,560 before manifest resolution only for UI estimates
- Qwen3-8B hidden size comes from the resolved manifest; approximate planning may use 4,096 before manifest resolution only for UI estimates

Prefill cost replaces decode `sequence` with the prompt chunk length and uses the same formula per chunk boundary.

`stage_compute_ms` comes from:

1. Measured warm-up for that stage and model revision when available.
2. Otherwise a conservative estimate from layer count and the newest local compute proxy, marked as estimated.

The planner minimizes `placement_cost_ms` among feasible plans. Feasibility is checked before cost.

## Rejection thresholds

### Hard rejects for any deployment

- Required backend or data type unsupported.
- Required direct control path from coordinator to a selected peer missing.
- Required adjacent-stage direct link missing.
- Any required link measurement expired or missing when the plan needs that link.
- Available memory below the reservation request at commit time.

### Hard rejects for continuous-layer pipeline hops

A hop is rejected when any of these hold after age adjustment:

| Metric | Threshold |
|---|---|
| One-way delay | `> 80 ms` |
| Directional bandwidth toward the next stage | `< 10 Mbps` |
| Stability score | `< 50` |
| Delay measurement age | expired |
| Bandwidth measurement age | expired |

### Maximum WAN stage count

- Maximum stages in one continuous-layer pipeline: **3**
- Single-node counts as 1 stage.
- Full-model replicas are independent one-stage deployments and are not limited by this stage count.
- A developer test override may force two stages on one LAN for proof work. That override is not the default planner.

### Mode preference

The planner still checks modes in this order:

1. Single node when the complete model and KV cache fit.
2. Full-model replicas when throughput is the goal and complete copies fit.
3. Continuous-layer pipeline only when capacity requires it or a measured lower cost beats replicas for the requested workload.

A pipeline plan whose estimated decode cost is worse than a feasible single-node plan is rejected. A pipeline plan may beat nothing when single-node and replica modes are infeasible because of memory.

## Concurrency and fairness

- At most one bandwidth benchmark stream runs on a node at a time.
- At most one delay measurement runs per peer pair at a time.
- Benchmark work must not starve control heartbeats.
- Active activation transfers preempt new bandwidth benchmarks.
- Failed benchmarks back off with capped exponential delay starting at 5 s and capping at 5 minutes.

## GUI requirements

The dashboard shows:

- Local CPU, memory, disk, and GPU summary from the newest capability report.
- Each connected peer's advertised hardware summary when received.
- Per-peer delay, both bandwidth directions, stability score, and measurement age state.

Values are labeled measured, stale, expired, or unavailable. The GUI does not invent placeholder speeds.

## Ownership

| Concern | Owner |
|---|---|
| Wire benchmark messages and streams | `mesh-net` |
| Storage-neutral report and measurement types | `mesh-core` |
| Local discovery and compute proxy | `mesh-hardware` |
| Scheduling probes and storing peer-facing snapshots | `mesh-node` |
| Cost formula and reject checks | `mesh-inference` Placement Planner |
| Rendering reports | `mesh-app` |

## Out of scope

- Public internet speed-test services.
- Assumed symmetric bandwidth.
- Clock-synchronized true one-way latency.
- Remote tensor-parallelism suitability scoring beyond the existing rejection of that mode.
- Persistent historical time-series beyond the newest valid measurement per peer and direction.
