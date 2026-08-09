# ADR-0009: Stable QUIC Identity and Invitations

| Field | Value |
|---|---|
| Status | Accepted |
| Date | 2026-08-09 |
| Owners | Architecture discussion |

## Context

A peer has no certificate authority or public lookup service. It still needs a stable identity across restarts and one self-contained invitation that lets a new PC contact and identify an existing peer.

## Decision

Generate one self-signed ECDSA P-256 certificate with `rcgen` when a node is created. Persist its PKCS#8 private key and certificate DER. Use the SHA-256 digest of the complete certificate DER as the 32-byte `NodeId`.

Use mutual QUIC TLS. During first enrollment, accept only the inviter certificate whose digest matches the invitation. During later connections, accept only certificates matching stored peer records. Hostname and public-certificate-authority validation do not identify mesh peers.

Encode `EnrollmentInviteV1` with Protobuf, then unpadded Base64 URL encoding. Use `mesh1:<payload>` as the canonical text. A `.mesh-invite` file contains that UTF-8 text. A `mesh://enroll/<payload>` URI and QR code carry the same payload.

Every invitation contains a random 16-byte enrollment ID and random 32-byte secret. The inviter binds first use to the joining certificate. Another certificate cannot reuse it. The default expiry is 30 minutes.

Canonical contract: [Enrollment contract](../architecture/onboarding/enrollment-contract.md)

## Rejected: random Node ID unrelated to QUIC identity

A separate random identity needs another signed binding to the TLS certificate. Deriving the Node ID from the persisted certificate removes that second identity mechanism.

## Rejected: public certificate authority

Public certificates identify internet names. Peers may have changing addresses and no domain name. A public certificate authority also adds an external enrollment dependency.

## Rejected: short numeric invitation

Without a public lookup service, a short code cannot locate the inviter or carry its identity and candidates. The invitation must be self-contained.

## Rejected: certificate rotation in the first version

Automatic rotation adds a signed transition and conflict rules. The first version keeps one certificate until the user explicitly resets or migrates identity. Rotation can be designed later without weakening the first binding.

## Consequences

- Resetting identity creates a new Node ID and requires enrollment again.
- The certificate and private key must be restored together.
- Invitations are longer than a short code but work as text, files, links, and QR codes.
- A leaked unused invitation can be used until it expires; one-time binding limits reuse.
- No invitation or certificate can make an unreachable peer reachable through CGNAT.
