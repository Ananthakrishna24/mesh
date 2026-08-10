# Peer-Record Merge Rules

| Field | Value |
|---|---|
| Status | Accepted |
| Canonical for | Peer Store merge, candidate expiry, offline retention, stale capabilities, and update frequency |
| Parent | [Node modules](../system/node-modules.md) |
| Decision | [ADR-0014: Peer-record merge rules](../../decisions/0014-peer-record-merge-rules.md) |
| Related | [Direct peer connection](direct-connection.md) |
| Related | [Persistent state](../system/persistent-state.md) |
| Related | [Control protocol](../protocol/control-protocol.md) |
| Related | [Network benchmark and placement cost](network-benchmark.md) |
| Implements gate | A10 |

## Boundary

Every node keeps its own Peer Store. There is no central peer database and no multi-hop gossip flood in the first implementation. A node learns peer records from:

1. Direct enrollment and reconnect handshakes.
2. `WELCOME.known_peers`.
3. `PeerUpdate` messages from currently connected peers.
4. Local observations (successful dial address, disconnect time, local capability refresh).

Merge rules decide what is kept, what expires, and what is overwritten. They do not create authority beyond certificate-bound Node IDs.

## Record model

Logical durable peer record:

```text
PeerRecord
├── node_id                          # immutable key
├── display_name
├── certificate_der                  # must hash to node_id
├── candidates[]
│   ├── kind
│   ├── address
│   ├── priority
│   ├── observed_at_unix_ms
│   ├── expires_at_unix_ms?
│   ├── source_node_id
│   └── reachability: Unknown | Reachable | Unreachable
├── last_successful_address?
├── last_seen_unix_ms?
├── first_seen_unix_ms
├── capability?
│   ├── report
│   └── accepted_at_unix_ms
├── record_updated_at_unix_ms
└── origin: LocalSelf | DirectPeer | IndirectPeer
```

Wire `PeerRecord` may carry a subset. Receivers fill local-only fields themselves. Unknown newer fields are ignored per the control-protocol compatibility rules.

`LocalSelf` is never accepted from the network for the local Node ID.

## Identity invariants

1. `node_id == SHA-256(certificate_der)`.
2. A peer record with a mismatched certificate digest is rejected and not stored.
3. Once a Node ID is stored, a different certificate for that Node ID is rejected.
4. Display name changes are allowed; they do not change identity.
5. Leaving a mesh and resetting identity creates a new Node ID; old records are deleted with mesh state.

## Last-writer and field ownership

No single wall-clock owns the whole record. Fields merge by owner:

| Field | Owner | Merge rule |
|---|---|---|
| `certificate_der` / `node_id` | Enrollment identity | Immutable after first accepted insert |
| `display_name` | Subject peer | Newer `record_updated_at_unix_ms` from the subject or a direct session with that subject wins |
| `candidates` | Subject peer plus local observations | Union by address, then per-candidate rules below |
| `last_successful_address` | Local observer only | Written only by the local node after a successful handshake to that peer |
| `last_seen_unix_ms` | Local observer only | Max of previous local value and local now when a direct session is alive or completes handshake |
| `capability` | Subject peer | Replace only when the incoming report has a newer `collected_at_unix_ms` |
| `record_updated_at_unix_ms` | Writer of the accepted change | Set to the local receive/apply time for local fields; for subject-authored fields use max(local apply time, sender-supplied update time when present) |

If two subject-authored values conflict and timestamps are equal, keep the lexicographically larger canonical encoding of the field bytes. This is only a deterministic tie-break.

Indirect peers (`WELCOME.known_peers` or `PeerUpdate` about a third node) may introduce a never-seen Node ID and may refresh candidates and display name. They must not:

- overwrite a directly verified certificate with different bytes,
- clear `last_successful_address`,
- move `last_seen_unix_ms` backward,
- replace a fresher capability with an older one.

## Candidate merge

Candidates are keyed by normalized `SocketAddr` (`ip`, `port`). IPv6-mapped IPv4 addresses normalize to IPv4 before comparison.

For each address:

1. If only one side has it, keep it when it is not expired.
2. If both sides have it, keep the copy with the newer `observed_at_unix_ms`.
3. On equal `observed_at_unix_ms`, keep the higher-priority kind; if still tied, keep the higher explicit priority value.
4. `reachability`:
   - local successful dial or accept through that address → `Reachable` and refresh `observed_at_unix_ms`,
   - local failed dial after a complete attempt budget → `Unreachable` for that address only,
   - remote gossip never upgrades local `Unreachable` to `Reachable`,
   - remote gossip may add `Unknown` addresses.
5. Kind `PeerObserved` is written only from a direct peer's observed remote address for the other endpoint of that session, or from an introduction message in hole punching.
6. Kind `Manual` is local configuration. Remote peers may rediscover the same address under another kind; both may coexist until expiry, but dialing deduplicates by address.
7. Maximum candidates stored per peer: **32**, matching the invitation and `HELLO` limit. When trimming, drop expired first, then `Unreachable`, then oldest `observed_at_unix_ms`, then lowest priority.

### Candidate expiry

| Kind | Default lifetime from `observed_at_unix_ms` unless `expires_at_unix_ms` is set |
|---|---|
| `GlobalIpv6` | 24 hours |
| `PublicIpv4` | 24 hours |
| `RouterMapping` | mapping lease end; if unknown, 2 hours |
| `Manual` | no automatic expiry until the user removes it |
| `PeerObserved` | 30 minutes |
| `LocalNetwork` | 24 hours |

Rules:

- A candidate with `expires_at_unix_ms <= now` is expired.
- Expired candidates are not advertised in new invitations, `HELLO`, `WELCOME`, or `PeerUpdate`.
- Expired candidates may remain on disk up to the offline retention window as tombstone-free historical data only if marked expired; dialers skip them.
- Successful use refreshes `observed_at_unix_ms` and, for `RouterMapping`, should refresh lease metadata from the mapper.
- Invitation embedded candidates are snapshots. They are valid for dialing during that enrollment attempt even if the inviter later expires them locally, until the invitation itself expires.

## Capability handling

Capability reports follow [Network benchmark and placement cost](network-benchmark.md) age windows for placement:

| Age of `collected_at_unix_ms` | Peer Store label | Behavior |
|---|---|---|
| `< 5 minutes` | Fresh | Show and use |
| `5..30 minutes` | Stale | Show with stale marker; placement applies stale multipliers |
| `> 30 minutes` | Expired | Keep last report for UI history until replaced; placement must not use it for new plans |
| Missing | Unknown | Node is networking-only until a report arrives |

Merge:

- Accept a capability only from the subject peer over a direct authenticated session, or from a direct session's post-handshake capability exchange with that peer.
- Gossip about a third peer may carry a capability summary only as non-authoritative cache. First implementation: **do not** accept third-party capability bodies into durable Peer Store. Third parties may still advertise addresses.
- A newer `collected_at_unix_ms` replaces the stored report entirely. Fields are not mixed across reports.
- Hardware disappearance (empty GPU list) is valid when reported by the subject.

Link measurements are not peer-record fields. They remain local Network Profiler state keyed by `(local_node_id, peer_node_id)` and follow A07 ageing.

## Offline retention

| State | Retention |
|---|---|
| Peer seen and enrolled | Keep indefinitely while the local node remains in the mesh |
| Peer offline | Keep record, candidates (subject to expiry), and last capability |
| Peer unreachable for dialing | Keep identity; continue backoff using non-expired candidates |
| User leaves mesh / reset identity | Delete all peer records in the same transaction as membership clear |
| Corrupted certificate mapping | Delete that peer row and surface a recovery error |

There is no automatic deletion of enrolled peers after an idle timer in the first version. Address expiry already prevents dialing dead paths. A later decision may add explicit “remove peer” UI.

## Merge conflicts

Conflict classes and outcomes:

1. **Same Node ID, different certificate:** reject incoming; keep stored; log identity conflict; do not connect.
2. **Self Node ID received from network:** ignore.
3. **Display name thrash:** last subject-authored timestamp wins; UI may show the accepted name only.
4. **Candidate address claimed by two kinds:** store one row per address; winning metadata follows candidate merge.
5. **Indirect older snapshot after direct fresh data:** ignore older subject fields; still union any new non-expired addresses.
6. **Two indirect introductions with no direct session yet:** union candidates; prefer newer display name timestamp; capability remains empty until direct exchange.
7. **Concurrent enrollment of two new peers:** independent upserts; no global lock across nodes; each local transaction remains atomic.

All durable multi-row changes stay inside one `mesh-store` transaction as required by the persistent-state contract.

## Update frequency

### Local publish

A node sends `PeerUpdate` to connected peers when:

| Event | Minimum delay before another update | Payload |
|---|---|---|
| Successful enrollment of a new peer | immediate | new peer identity + candidates |
| Local candidate set changes materially | 5 seconds coalesce | local self record as seen by peers, or affected peer |
| Display name change | immediate | self record |
| Periodic refresh while connected | 10 minutes | self candidates still advertised |
| Reconnect after session loss | immediate after handshake | self candidates |

Material candidate changes:

- add or remove a non-expired address,
- kind change for an address,
- `RouterMapping` external port or IP change,
- manual candidate edit.

### Local gather

| Task | Frequency |
|---|---|
| Local interface candidate refresh | on start, on routing/interface change if observed, else every 10 minutes |
| Router mapping renew | per [NAT and router mapping](nat-router-mapping.md) |
| Capability refresh publish | on material hardware change or every 10 minutes while idle enough |
| Peer dial backoff | capped exponential with jitter from the direct-connection state machine |

### Receive limits

- Maximum `PeerUpdate` peers per message: **64**.
- Maximum candidates per peer: **32**.
- Apply updates on the storage worker; coalesce bursts received within 1 second.
- Ignore updates that fail validation without closing the session unless identity forgery is detected on the authenticated peer itself.

## Dialing and advertisement filters

When building dial lists, invitations, `HELLO`, `WELCOME`, or `PeerUpdate`:

1. Drop expired candidates.
2. Drop unspecified addresses and port `0`.
3. Drop local-only loopback candidates from invitations and remote advertisements unless the peer is known to be on-loopback test mode.
4. Prefer order: `GlobalIpv6`, `PublicIpv4`, `RouterMapping`, `Manual`, `PeerObserved`, `LocalNetwork`, then higher explicit priority, then newer `observed_at_unix_ms`.
5. Include at most 32 candidates after filtering.

## Persistence mapping

`mesh-store` must persist enough metadata to apply these rules after restart:

- existing peer identity columns,
- candidate JSON extended with `observed_at_unix_ms`, optional `expires_at_unix_ms`, `source_node_id`, and `reachability`,
- `last_successful_address`,
- `last_seen_unix_ms`,
- `first_seen_unix_ms`,
- optional capability blob + `accepted_at_unix_ms`,
- `record_updated_at_unix_ms`.

Schema migration is required when implementing P04 if the current `peers` table lacks these columns. Until migration runs, in-memory defaults may treat missing timestamps as `now` at read time only for records written by older versions, then rewrite on next upsert.

## Ownership

| Concern | Owner |
|---|---|
| Pure merge functions and record types | `mesh-core` |
| Durable upsert/list/migrate | `mesh-store` |
| When to send `PeerUpdate` and what was observed on the wire | `mesh-net` / `mesh-node` |
| GUI labels for stale/expired/offline | `mesh-app` via snapshots |

## Non-goals

- CRDTs across the full mesh.
- Epidemic gossip beyond currently connected peers.
- Automatic permanent deletion of idle enrolled peers.
- Trusting third-party capability or benchmark claims.
- Relays for peers that remain unreachable after expiry and failed dials.
