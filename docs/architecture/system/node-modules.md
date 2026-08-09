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
│ M05 Hardware      │ M06 Node State    │ M07 Job Manager    │
│ Scanner           │                   │                    │
│ Local devices     │ Current local     │ Creates or accepts │
│ and capabilities  │ state             │ jobs               │
├───────────────────┴───────────────────┴────────────────────┤
│ M08 GPU Worker                                             │
│ Runs assigned compute on one selected local backend.       │
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

## M07 — Job Manager

**Owns:** job creation and job participation.

When local software creates a job, it:

1. Reads peer capabilities.
2. Selects possible workers.
3. Splits work using the job-specific strategy.
4. Sends work directly.
5. Tracks progress.
6. Combines results.

When another peer owns the job, it validates the offer and passes accepted work to the GPU Worker.

**Rule:** the creator owns only that job.

## M08 — GPU Worker

**Owns:** local compute execution.

It selects a supported backend, reserves local resources, runs assigned work, and returns progress or a result to the Job Manager.

**Rule:** CUDA and Metal remain separate native backends behind one Rust interface. Do not force both through a slower common GPU API.
