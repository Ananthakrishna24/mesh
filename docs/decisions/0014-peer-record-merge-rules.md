# ADR-0014: Peer-Record Merge Rules

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-08-10 |
| Owners | Architecture discussion |
| Gate | A10 |

## Context

After enrollment, every node stores peers and candidates locally. Invitations, `WELCOME.known_peers`, reconnects, router mappings, and later hole-punch introductions all produce competing address lists. Without merge rules, nodes overwrite fresh direct state with stale gossip, keep dead addresses forever, or accept third-party capability claims. P04 automatic connectivity needs deterministic expiry, retention, and update frequency before peer exchange grows beyond the inviter snapshot.

## Decision

Accept the Peer Store contract in [Peer-record merge rules](../architecture/networking/peer-record-merge.md).

Key rules:

- Certificate-bound Node ID is immutable; certificate mismatch rejects the update.
- Field ownership is split: subject-authored name/candidates/capabilities versus local-only last-seen and last-successful address.
- Candidates merge by normalized address with explicit lifetimes per kind.
- Third-party gossip may introduce identities and addresses but must not authoritatively replace fresher direct state or install third-party capability bodies in v1.
- Enrolled peers are retained offline until the user leaves the mesh; addresses expire instead.
- `PeerUpdate` is coalesced and rate-limited; periodic self refresh is 10 minutes.

## Rejected: last-write-wins on the entire peer blob

A single timestamp over the whole record lets an old indirect snapshot erase a local successful dial address or a newer capability. Per-field ownership avoids that.

## Rejected: CRDT gossip mesh

Full-mesh CRDTs and epidemic gossip add complexity before the first internet enrollment path works. Connected-peer exchange plus inviter snapshots is enough for the first small deployments.

## Rejected: automatic deletion of idle peers

Idle deletion surprises users when a laptop sleeps for days. Candidate expiry already stops useless dials. Explicit remove/leave actions remain the destructive paths.

## Consequences

- `mesh-core` peer and candidate types gain observation, expiry, reachability, and local metadata fields.
- `mesh-store` needs a schema migration when P04 persists the extended fields.
- `mesh-net` / `mesh-node` must call pure merge helpers instead of blind upsert replacement.
- Capability freshness labels stay aligned with A07 measurement age windows.
- Control proto can add optional candidate metadata fields compatibly within protocol major 1 when implementation needs them on the wire.
