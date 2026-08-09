# Enrollment Contract

| Field | Value |
|---|---|
| Status | Accepted |
| Canonical for | Invitation contents, automatic enrollment states, and user-facing failures |
| Parent | [Desktop onboarding](README.md) |
| Network flow | [Direct peer connection](../networking/direct-connection.md) |

## Goal

Enroll a PC with one invitation and no manual networking commands when the internet path allows automatic direct access.

## Node identity

The first startup creates:

- One self-signed ECDSA P-256 certificate.
- Its PKCS#8 private key.
- A 32-byte `NodeId` equal to SHA-256 of the complete certificate DER.

Certificate parameters are fixed for the first version:

- Signature algorithm: ECDSA P-256 with SHA-256.
- One self-signed leaf certificate and no chain.
- Subject common name: `mesh-node`.
- Valid from 24 hours before creation until 20 years after creation.
- No IP address or public DNS identity is placed in the certificate.
- The same certificate is used for Quinn client and server roles.

The certificate, key, and Node ID are persisted together and reused after restart. They are not regenerated during normal startup.

Every QUIC connection uses mutual TLS:

1. Rustls verifies proof of possession during the TLS handshake.
2. The certificate must be within its validity interval.
3. During enrollment, the joining node accepts only the inviter certificate whose SHA-256 digest equals the inviter Node ID in the invitation.
4. The inviter reads the joining certificate, derives its Node ID, and binds the invitation to it.
5. After enrollment, each side accepts only certificates matching its persisted Peer Store.
6. DNS names and public certificate authorities do not identify mesh peers.
7. An explicit identity reset creates a new Node ID and requires enrollment again.

The inviter must accept an unknown joining certificate provisionally at the TLS layer because the joining Node ID is not in Peer Store yet. This provisional connection may process only one bounded `HELLO`. It cannot change durable mesh state, query resources, or start work until the invitation secret binds that certificate and the enrollment transaction commits.

Canonical decision: [ADR-0009](../../decisions/0009-quic-identity-and-invitations.md)

## Invitation lifecycle

1. A connected peer creates an invitation.
2. It stores the invitation ID, a hash of its secret, expiry, and `PENDING` state.
3. The invitation records the inviter's current reachable candidates.
4. The user moves the invitation to the new PC.
5. The new PC presents the invitation ID and secret inside its first TLS-protected `HELLO`.
6. The inviter atomically binds the invitation to the joining certificate's Node ID.
7. A retry from that same Node ID is allowed until enrollment completes or the invitation expires.
8. A different Node ID is rejected.
9. The invitation becomes `CONSUMED` after permanent peer state is written.
10. A new invitation is generated when candidates change materially or the invitation expires.

The default expiry is 30 minutes. The inviter's clock decides expiry. Successful enrollment and invitation state changes are committed in one local database transaction.

## Invitation data

The accepted logical schema is:

```proto
message EnrollmentInviteV1 {
  uint32 format_version = 1;          // exactly 1
  uint32 protocol_major = 2;
  uint32 protocol_minor_min = 3;
  uint32 protocol_minor_max = 4;
  bytes mesh_id = 5;                  // exactly 16 bytes
  bytes inviter_node_id = 6;          // exactly 32 bytes
  string inviter_name = 7;            // 1..128 UTF-8 bytes
  bytes enrollment_id = 8;            // exactly 16 random bytes
  bytes enrollment_secret = 9;        // exactly 32 random bytes
  int64 expires_at_unix_ms = 10;
  repeated EndpointCandidate candidates = 11; // at most 32
}
```

Validation happens before any connection attempt:

- The decoded Protobuf payload is at most 64 KiB.
- Every fixed-size identifier has its exact length.
- The protocol range is ordered and includes the current major version.
- The expiry is present. The joining PC may warn when its local clock says expired, but the inviter makes the authoritative expiry decision.
- Candidate ports are nonzero and candidate addresses are valid.
- Duplicate candidates are removed.

The user does not look up an IP address, port, Mesh ID, Node ID, or certificate digest during normal enrollment.

## Exact encoding

1. Encode `EnrollmentInviteV1` with Prost Protocol Buffers.
2. Encode those bytes with unpadded Base64 URL encoding.
3. Prefix the result with `mesh1:` for copied text.

All GUI inputs normalize to the same Protobuf bytes:

| Form | Encoding |
|---|---|
| Copied text | `mesh1:<base64url>` |
| File | UTF-8 `mesh1:<base64url>` in a `.mesh-invite` file |
| URI | `mesh://enroll/<base64url>` |
| QR code | The same `mesh://enroll/<base64url>` URI |

Whitespace around copied text or file content is ignored. Whitespace inside the payload is invalid. Unknown invite format versions are rejected rather than guessed.

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
