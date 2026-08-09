# Enrollment Contract

| Field | Value |
|---|---|
| Status | Accepted behavior; exact encoding proposed |
| Canonical for | Invitation contents, automatic enrollment states, and user-facing failures |
| Parent | [Desktop onboarding](README.md) |
| Network flow | [Direct peer connection](../networking/direct-connection.md) |

## Goal

Enroll a PC with one invitation and no manual networking commands when the internet path allows automatic direct access.

## Invitation lifecycle

1. A connected peer creates an invitation.
2. The invitation records the inviter's current reachable candidates.
3. The user moves the invitation to the new PC.
4. The new PC uses it once to join.
5. The invitation expires after a configured time or successful use.
6. A new invitation is generated when addresses change or the invitation expires.

Expiration and one-time use avoid confusing retries with stale connection information. Exact durations remain proposed until connection tests exist.

## Invitation data

| Field | Obtained from | User action |
|---|---|---|
| Format version | Application constant | None |
| Protocol version range | Application build | None |
| Mesh ID | Existing local state | None |
| Inviter Node ID | Existing local state | None |
| Inviter display name | Existing local state | None |
| IPv6 candidates | Address Candidate Collector | None |
| Public IPv4 candidates | Address Candidate Collector | None |
| Router-mapped candidates | Automatic UPnP, NAT-PMP, or PCP result | None |
| Manually configured candidate | Advanced network settings | Only after automatic failure |
| QUIC certificate fingerprint | Existing local identity | None |
| Enrollment ID | Generated for this invitation | None |
| Expiry | Generated for this invitation | None |

The user does not look up an IP address, port, Mesh ID, Node ID, or certificate fingerprint during normal enrollment.

## Proposed logical form

```rust
struct EnrollmentInvite {
    format_version: u16,
    protocol_min: u16,
    protocol_max: u16,
    mesh_id: MeshId,
    inviter_node_id: NodeId,
    inviter_name: String,
    certificate_fingerprint: String,
    enrollment_id: EnrollmentId,
    expires_at: Timestamp,
    candidates: Vec<EndpointCandidate>,
}
```

The exact serialized encoding remains part of the wire-format decision. The GUI supports text, file, and URI wrappers around the same encoded payload.

## Data shared after connection

After the QUIC connection succeeds, peers exchange:

- Protocol version.
- Mesh ID and Node ID.
- Display name.
- Reachable candidate addresses.
- Current connection state.
- Operating system and CPU architecture.
- CPU and memory summary.
- GPU vendor, model, backend, memory, and runtime capability.
- Available disk and model-cache summary.
- Known peer records.
- Network benchmark results produced between those peers.
- Model-provider capability names when enabled.

They do not exchange:

- Provider access tokens.
- Personal files.
- Unselected model data.
- Arbitrary filesystem paths.
- Another application's data.

## Enrollment state machine

```text
NOT_ENROLLED
      │
      ├── Create ──▶ CREATING_IDENTITY
      │                    │
      │                    ▼
      │               PREPARING_NODE
      │                    │
      │                    ▼
      │                 ENROLLED
      │
      └── Join ────▶ READING_INVITE
                           │
                           ▼
                     PREPARING_NODE
                           │
                           ▼
                    CONNECTING_INVITER
                           │
                           ▼
                       HANDSHAKING
                           │
                           ▼
                    SYNCING_PEER_LIST
                           │
                           ▼
                  CONNECTING_OTHER_PEERS
                           │
                           ▼
                    SHARING_CAPABILITY
                           │
                           ▼
                       ENROLLED
```

Every state publishes:

- Stable state name.
- Simple progress sentence.
- Optional technical details.
- Whether cancellation is safe.
- Recommended recovery after failure.

## Automatic preparation

`PREPARING_NODE` performs independent tasks in parallel when possible:

```text
├── Load or create node identity
├── Scan CPU, memory, disks, and GPUs
├── Bind the QUIC UDP port
├── Gather local and global IPv6 addresses
├── Attempt router port mapping
└── Load provider configuration
```

Enrollment waits only for tasks required to contact the inviter. Slow hardware details may complete while the direct connection is being established.

## Progress events

The node runtime emits typed events. The GUI turns them into simple text.

Examples:

| Runtime event | User text |
|---|---|
| `IdentityReady` | Created this PC's identity |
| `GpuDetected` | Detected NVIDIA or Apple GPU |
| `PortBound` | Opened the local connection port |
| `RouterMappingCreated` | Router connection prepared automatically |
| `InviterConnected` | Connected to the existing PC |
| `PeerSnapshotReceived` | Received the known PC list |
| `PeerConnected` | Connected directly to another PC |
| `CapabilityPublished` | Shared this PC's hardware details |
| `EnrollmentComplete` | This PC is ready |

Technical logs remain available separately. UI text is not parsed from log messages.

## Automatic recovery order

When the inviter cannot be reached:

1. Retry the best candidate briefly.
2. Try every remaining IPv6 candidate.
3. Try every remaining IPv4 and router-mapped candidate.
4. Refresh local router mapping.
5. Ask the user to regenerate the invitation on the inviter.
6. Show guided manual port forwarding only when every automatic method fails.

The GUI does not retry forever. It explains the current action and provides **Try again** and **Show technical details**.

## Guided failures

### Invitation expired

Message:

> This invitation has expired. On a connected PC, choose “Add another PC” and create a new invitation.

Primary action: **Open invitation**.

### Protocol version mismatch

Message:

> The PCs are running incompatible application versions. Update the older application and try again.

Primary action: **Show version details**.

### No direct route

Message:

> The two routers did not allow a direct connection automatically.

Primary action: **Try automatic setup again**.

Secondary advanced action: **Show manual router steps**.

The manual screen shows:

- Local UDP port.
- Required protocol: UDP.
- Detected local address.
- Where to paste the resulting public address.
- A new invitation button after configuration.

Do not promise that manual forwarding works through provider-level CGNAT.

### Operating-system firewall

Message:

> This PC's firewall may be blocking the mesh connection. Allow the Mesh application to receive UDP connections, then try again.

The application may request the normal operating-system firewall permission. It does not silently disable the firewall.

### Inviter went offline

Message:

> The PC that created this invitation is offline. Start Mesh on that PC or create an invitation from another connected PC.

### Hardware scan warning

Enrollment may continue without a supported GPU. The dashboard marks the node as networking-only and explains why CUDA or Metal did not initialize.

### Some peers remain unreachable

Enrollment succeeds when the inviter connection and mesh handshake succeed. The dashboard lists other unreachable peers separately. The node must not report a complete full mesh when only part of the peer graph connected.

### Provider access missing

Enrollment itself succeeds. Provider access is requested only when a selected model needs it.

## Concurrent invitations

Any enrolled peer may create an invitation. Enrollment IDs prevent duplicate completion events when the same invitation is retried.

Two PCs joining at once are independent. Peer Store merges their records using the accepted peer-record rules once those rules are finalized.

## Cancellation

The user may cancel before the mesh handshake completes.

Cancellation:

- Stops connection attempts.
- Removes temporary enrollment state.
- Keeps a newly generated Node ID for reuse unless the user resets the app.
- Removes an unused temporary router mapping when the application created one.
- Does not delete valid hardware scan results.

After the mesh handshake writes permanent mesh state, leaving the mesh is a separate explicit action.

## Completion invariant

The GUI shows **This PC is ready** only when:

1. Stable local identity exists.
2. Mesh ID is persisted.
3. At least one inviter handshake completed.
4. Peer Store was persisted.
5. Hardware capability state exists, even if no supported GPU was found.
6. The node runtime is active.

Connections to every known peer are not required for enrollment completion. Their individual states remain visible on the dashboard.
