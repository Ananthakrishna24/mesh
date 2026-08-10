# Architecture Decisions

Read accepted decisions before changing a related subsystem.

| ID | Decision | Status |
|---|---|---|
| [ADR-0001](0001-direct-quic-transport.md) | Direct QUIC transport with Quinn | Accepted |
| [ADR-0002](0002-native-gpu-backends.md) | Native CUDA and Metal backend paths | Accepted boundary; libraries proposed |
| [ADR-0003](0003-wan-inference-modes.md) | WAN inference modes and local reservations | Accepted |
| [ADR-0004](0004-provider-backed-model-distribution.md) | Immutable provider-backed model distribution | Accepted |
| [ADR-0005](0005-native-desktop-onboarding.md) | Native Rust desktop onboarding | Accepted |
| [ADR-0006](0006-windows-nvidia-required.md) | Windows NVIDIA CUDA is a required target | Accepted |
| [ADR-0007](0007-qwen3-first-model-family.md) | Qwen3 4B and 8B are the first test models | Accepted |
| [ADR-0008](0008-protobuf-control-protocol.md) | Protobuf control protocol with explicit framing | Accepted |
| [ADR-0009](0009-quic-identity-and-invitations.md) | Stable QUIC identity and self-contained invitations | Accepted |
| [ADR-0010](0010-sqlite-state-and-native-credentials.md) | SQLite state and native provider credentials | Accepted |
| [ADR-0011](0011-fixed-activation-frame.md) | Fixed activation tensor frame | Accepted |
| [ADR-0012](0012-network-benchmark-and-placement-cost.md) | Network benchmark and placement cost | Accepted |
| [ADR-0013](0013-nat-router-mapping-crates.md) | NAT and router mapping crates | Accepted |
| [ADR-0014](0014-peer-record-merge-rules.md) | Peer-record merge rules | Accepted |
| [ADR-0015](0015-provider-manifest-download-cache.md) | Provider manifest, partial download, and cache policy | Accepted |
| [ADR-0016](0016-tokenizer-sampling-kv-cache.md) | Tokenizer, sampling, and KV-cache contracts | Accepted |



A new decision must state its context, selected approach, rejected approaches, and consequences. A replacement decision must link to the decision it replaces.
