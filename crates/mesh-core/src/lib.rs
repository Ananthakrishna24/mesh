mod error;
mod hardware;
mod ids;
mod identity;
pub mod invite;
mod peer;
pub mod protocol;
mod resource;
mod model;

mod ui;

pub use error::{CoreError, CoreResult};
pub use hardware::{
    BANDWIDTH_REJECT_BPS, BandwidthMeasurement, CapabilityReport, ComputeProxy,
    DEFAULT_BANDWIDTH_PAYLOAD_BYTES, DELAY_REJECT_ONE_WAY_MS, DelayMeasurement, GpuBackendKind,
    GpuDeviceInfo, LinkMeasurement, MAX_BANDWIDTH_PAYLOAD_BYTES, MAX_WAN_PIPELINE_STAGES,
    MEASUREMENT_FRESH_MS, MEASUREMENT_STALE_MS, MIN_BANDWIDTH_PAYLOAD_BYTES, MeasurementAgeState,
    STABILITY_PIPELINE_MIN, age_bandwidth_bps, age_delay_ms, format_bits_per_second, format_bytes,
    measurement_age_state, pipeline_hop_rejects, stability_score,
};
pub use ids::{DeploymentId, EnrollmentId, MeshId, NodeId, ReservationId};
pub use identity::{LocalIdentity, identity_matches};
pub use invite::{
    INVITE_PREFIX, InvitationText, build_invite, candidates_from_proto, candidates_to_proto,
    decode_invitation_text, encode_invitation_text, secret_digest, validate_invite,
};
pub use peer::{
    CandidateKind, CandidateReachability, EndpointCandidate, InvitationRecord, InvitationState,
    MAX_PEER_CANDIDATES, PeerMergeError, PeerRecord, PeerRecordOrigin, PeerSummary,
    candidate_is_advertisable, filter_advertised_candidates, merge_candidates, merge_peer_records,
    normalize_candidate_address, sort_candidates_for_dial,
};
pub use protocol::{
    MAX_CONTROL_FRAME_BYTES, PROTOCOL_MAJOR, PROTOCOL_MINOR, PROTOCOL_MINOR_MIN, capability_digest,
    now_unix_ms, proto, random_message_id,
};
pub use resource::{
    DEFAULT_COMMIT_LEASE_MS, DEFAULT_HOLD_LEASE_MS, GpuResourceAmount, LocalReservation, MAX_LEASE_MS,
    MIN_LEASE_MS, OFFER_TTL_MS, ReservationCommit, ReservationRelease, ReservationState,
    ReservationSummaryView, ReserveAccepted, ReserveRejected, ReserveRequest, ResourceAmount,
    ResourceCapacity, ResourceManagerView, ResourceOffer, ResourceQuery, clamp_lease_ms,
    current_time_ms, offer_expiry,
};
pub use model::{
    ADAPTER_QWEN3_DENSE, ADAPTER_QWEN3_DENSE_VERSION, CACHE_VOLUME_RESERVE_BYTES,
    CACHE_VOLUME_RESERVE_RATIO_DEN, CACHE_VOLUME_RESERVE_RATIO_NUM, CacheValidationState,
    DEFAULT_CACHE_MAX_BYTES, DISK_PREPARE_MARGIN_BYTES, ETAG_REVALIDATE_MS, MAX_SAFETENSORS_HEADER_BYTES,
    ModelCacheEntry, ModelCacheView, ModelFormat, ModelIdentity, ModelManifestRecord,
    ModelReference, ModelStoreView, PARTIAL_GRACE_MS, PROVIDER_HUGGINGFACE, ProviderAccessReport,
    ProviderAccessStatus, ProviderAuthMode, RANGE_MERGE_GAP_BYTES, is_full_commit_sha,
    manifest_cache_key, prepare_disk_margin, short_revision, volume_reserve_floor,
};

pub use ui::{
    AppScreen, ConnectivityRecovery, EnrollmentProgress, HardwareSummaryView, LinkSummaryView,
    LocalNodeSummary, ManualForwardingGuide, RecoveryAction, RuntimePhase, UiCommand, UiSnapshot,
};

