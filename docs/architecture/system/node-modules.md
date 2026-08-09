# Node Modules

| Field | Value |
|---|---|
| Status | Accepted foundation |
| Canonical for | Modules that run inside every PC |
| Parent | [Architecture overview](../README.md) |

Every PC runs the same modules. A module owns one clear responsibility.

## Module map

```text
┌────────────────────────────────────────────────────────────┐
│ M01 Node Connector                                         │
│ Starts the node and routes messages between modules.       │
├───────────────────┬───────────────────┬────────────────────┤
│ M02 Peer Store    │ M03 Address       │ M04 Direct Link    │
│                   │ Candidate         │ Manager            │
│ Known peers and   │ Collector         │ Active peer links  │
│ their addresses   │ Reachable paths   │ and reconnects     │
├───────────────────┼───────────────────┼────────────────────┤
│ M05 Hardware      │ M06 Node State    │ M07 Network        │
│ Scanner           │                   │ Profiler           │
│ Local devices     │ Current local     │ Link measurements  │
│ and capabilities  │ state             │                    │
├───────────────────┼───────────────────┼────────────────────┤
│ M08 Job Manager   │ M09 Local         │ M10 Model Store    │
│                   │ Resource Manager  │                    │
│ Job lifecycle     │ Local leases      │ Model cache        │
├───────────────────┴───────────────────┴────────────────────┤
│ M11 Inference Worker                                       │
│ Loads and runs one assigned model stage.                   │
└────────────────────────────────────────────────────────────┘
```

## M01 — Node Connector

**Owns:** node startup, module lifecycle, and message routing.

**Inputs:** local configuration and messages from Direct Link Manager.

**Outputs:** commands to local modules and messages for connected peers.

**Rule:** it is not a mesh master. Every PC runs its own connector.

## M02 — Peer Store

**Owns:** the local list of known peers.

Each peer record contains:

- Stable `NodeId`.
- Last known candidate addresses.
- Last successful direct address.
- Current connection state.
- Last seen time.
- Hardware capability summary.

**Rule:** each PC owns its local copy. There is no central peer database.

## M03 — Address Candidate Collector

**Owns:** addresses another PC may try.

It collects:

1. Local IPv4 and IPv6 addresses.
2. Globally reachable IPv6 addresses.
3. Public IPv4 and port mappings exposed by the router.
4. Manually configured public addresses.
5. Addresses observed by already-connected peers.

**Rule:** an address is only a candidate. A successful connection proves reachability.

## M04 — Direct Link Manager

**Owns:** the QUIC endpoint and direct peer sessions.

It:

1. Binds the UDP socket.
2. Accepts incoming connections.
3. Dials candidate addresses.
4. Performs the mesh handshake.
5. Removes duplicate connections.
6. Keeps active connections alive.
7. Reconnects after failure.
8. Coordinates peer-assisted hole punching.

Canonical flow: [Direct connection algorithm](../networking/direct-connection.md)

## M05 — Hardware Scanner

**Owns:** local hardware discovery.

The first report contains:

- Operating system and CPU architecture.
- CPU model and logical core count.
- Total and available system memory.
- GPU backend: `cuda` or `metal`.
- GPU vendor and model.
- Total and available GPU memory.
- Driver or runtime version.
- Features required by the compute backend.

Canonical backend plan: [GPU backends](../compute/gpu-backends.md)

## M06 — Node State

**Owns:** the current local view of the node.

It stores:

- Local identity and mesh identity.
- Hardware report.
- Direct connection states.
- Active jobs.
- Current resource usage.
- Local health.

**Rule:** other modules update state through explicit commands. They do not hold competing copies.

## M07 — Network Profiler

**Owns:** measured direct-link performance.

It records directional delay, bandwidth, stability, and recent measurement time for connected peers. Measurements are bounded so they do not interfere with active inference.

**Rule:** placement uses measured links, not assumed internet speed.

## M08 — Job Manager

**Owns:** generic job creation and participation.

For an inference deployment, it creates the temporary Inference Coordinator on the job owner and routes job messages to the owning local modules.

**Rule:** the creator owns only that job.

Canonical inference design: [Distributed LLM inference](../inference/README.md)

## M09 — Local Resource Manager

**Owns:** authoritative local resource offers, expiring reservations, commits, and releases.

It prevents separate job owners from assigning the same GPU memory or execution capacity.

**Rule:** a remote Placement Planner may propose work. Only this local module may reserve the node's resources.

## M10 — Model Store

**Owns:** immutable model artifacts, partial tensor downloads, validation, disk caching, loading, and eviction.

It follows a placement plan. It does not decide which layers belong on the node.

Canonical distribution flow: [Provider-backed model distribution](../inference/model-distribution.md)

## M11 — Inference Worker

**Owns:** local model-stage execution and that stage's KV cache.

It loads one assigned continuous layer range, runs CUDA or Metal operations, and sends activations to the next stage.

**Rule:** CUDA and Metal remain separate native backends behind one Rust interface. Do not force both through a slower common GPU API.
