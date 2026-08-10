mod error;
mod ids;
mod identity;
pub mod invite;
mod peer;
pub mod protocol;
mod ui;

pub use error::{CoreError, CoreResult};
pub use ids::{EnrollmentId, MeshId, NodeId};
pub use identity::{LocalIdentity, identity_matches};
pub use invite::{
    INVITE_PREFIX, InvitationText, build_invite, candidates_from_proto, candidates_to_proto,
    decode_invitation_text, encode_invitation_text, secret_digest, validate_invite,
};
pub use peer::{
    CandidateKind, EndpointCandidate, InvitationRecord, InvitationState, PeerRecord, PeerSummary,
};
pub use protocol::{
    MAX_CONTROL_FRAME_BYTES, PROTOCOL_MAJOR, PROTOCOL_MINOR, PROTOCOL_MINOR_MIN, capability_digest,
    now_unix_ms, proto, random_message_id,
};
pub use ui::{
    AppScreen, EnrollmentProgress, LocalNodeSummary, RuntimePhase, UiCommand, UiSnapshot,
};
