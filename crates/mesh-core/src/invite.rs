use std::net::SocketAddr;

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use prost::Message;
use sha2::{Digest, Sha256};

use crate::protocol::proto::{
    CandidateKind as ProtoCandidateKind, EnrollmentInviteV1, EndpointCandidate as ProtoCandidate,
};
use crate::{
    CandidateKind, CoreError, CoreResult, EndpointCandidate, EnrollmentId, MeshId, NodeId,
    PROTOCOL_MAJOR, PROTOCOL_MINOR, PROTOCOL_MINOR_MIN, now_unix_ms,
};

pub const INVITE_PREFIX: &str = "mesh1:";
const MAX_INVITE_BYTES: usize = 64 * 1024;
const MAX_CANDIDATES: usize = 32;
const MAX_NAME_BYTES: usize = 128;

#[derive(Debug, Clone, PartialEq)]
pub struct InvitationText {
    pub invite: EnrollmentInviteV1,
    pub text: String,
}

pub fn encode_invitation_text(invite: &EnrollmentInviteV1) -> CoreResult<String> {
    validate_invite(invite)?;
    let bytes = invite.encode_to_vec();
    if bytes.len() > MAX_INVITE_BYTES {
        return Err(CoreError::InvalidInvitation(
            "encoded invitation exceeds 64 KiB".to_owned(),
        ));
    }
    Ok(format!(
        "{INVITE_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(bytes)
    ))
}

pub fn decode_invitation_text(input: &str) -> CoreResult<EnrollmentInviteV1> {
    let trimmed = input.trim();
    let payload = if let Some(rest) = trimmed.strip_prefix(INVITE_PREFIX) {
        rest.trim()
    } else if let Some(rest) = trimmed.strip_prefix("mesh://enroll/") {
        rest.trim()
    } else {
        return Err(CoreError::InvalidInvitation(
            "invitation must start with mesh1: or mesh://enroll/".to_owned(),
        ));
    };

    if payload.chars().any(char::is_whitespace) {
        return Err(CoreError::InvalidInvitation(
            "invitation payload contains whitespace".to_owned(),
        ));
    }

    let bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| CoreError::InvalidInvitation("invalid base64url payload".to_owned()))?;
    if bytes.is_empty() || bytes.len() > MAX_INVITE_BYTES {
        return Err(CoreError::InvalidInvitation(
            "invitation payload size is invalid".to_owned(),
        ));
    }

    let invite = EnrollmentInviteV1::decode(bytes.as_slice())
        .map_err(|_| CoreError::InvalidInvitation("invalid invitation protobuf".to_owned()))?;
    validate_invite(&invite)?;
    Ok(invite)
}

pub fn validate_invite(invite: &EnrollmentInviteV1) -> CoreResult<()> {
    if invite.format_version != 1 {
        return Err(CoreError::InvalidInvitation(format!(
            "unsupported invite format version {}",
            invite.format_version
        )));
    }
    if invite.protocol_major != PROTOCOL_MAJOR {
        return Err(CoreError::InvalidInvitation(format!(
            "unsupported protocol major {}",
            invite.protocol_major
        )));
    }
    if invite.protocol_minor_min > invite.protocol_minor_max {
        return Err(CoreError::InvalidInvitation(
            "protocol minor range is unordered".to_owned(),
        ));
    }
    if invite.protocol_minor_min > PROTOCOL_MINOR || invite.protocol_minor_max < PROTOCOL_MINOR_MIN
    {
        return Err(CoreError::InvalidInvitation(
            "protocol minor range is incompatible".to_owned(),
        ));
    }
    MeshId::from_slice(&invite.mesh_id)?;
    NodeId::from_slice(&invite.inviter_node_id)?;
    EnrollmentId::from_slice(&invite.enrollment_id)?;
    if invite.enrollment_secret.len() != 32 {
        return Err(CoreError::InvalidInvitation(
            "enrollment secret must be 32 bytes".to_owned(),
        ));
    }
    if invite.inviter_name.trim().is_empty() || invite.inviter_name.len() > MAX_NAME_BYTES {
        return Err(CoreError::InvalidInvitation(
            "inviter name length is invalid".to_owned(),
        ));
    }
    if invite.expires_at_unix_ms <= 0 {
        return Err(CoreError::InvalidInvitation(
            "invitation expiry is missing".to_owned(),
        ));
    }
    if invite.candidates.is_empty() || invite.candidates.len() > MAX_CANDIDATES {
        return Err(CoreError::InvalidInvitation(
            "candidate list size is invalid".to_owned(),
        ));
    }
    for candidate in &invite.candidates {
        parse_proto_candidate(candidate)?;
    }
    Ok(())
}

pub fn secret_digest(secret: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(secret);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

pub fn candidates_from_proto(
    candidates: &[ProtoCandidate],
) -> CoreResult<Vec<EndpointCandidate>> {
    candidates.iter().map(parse_proto_candidate).collect()
}

pub fn candidates_to_proto(candidates: &[EndpointCandidate]) -> Vec<ProtoCandidate> {
    candidates
        .iter()
        .map(|candidate| ProtoCandidate {
            kind: proto_kind(candidate.kind) as i32,
            address: candidate.address.to_string(),
            priority: u32::from(candidate.priority),
        })
        .collect()
}

pub fn build_invite(
    mesh_id: MeshId,
    inviter_node_id: NodeId,
    inviter_name: impl Into<String>,
    enrollment_id: EnrollmentId,
    enrollment_secret: [u8; 32],
    expires_at_unix_ms: i64,
    candidates: &[EndpointCandidate],
) -> CoreResult<EnrollmentInviteV1> {
    let invite = EnrollmentInviteV1 {
        format_version: 1,
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor_min: PROTOCOL_MINOR_MIN,
        protocol_minor_max: PROTOCOL_MINOR,
        mesh_id: mesh_id.to_vec().into(),
        inviter_node_id: inviter_node_id.to_vec().into(),
        inviter_name: inviter_name.into(),
        enrollment_id: enrollment_id.to_vec().into(),
        enrollment_secret: enrollment_secret.to_vec().into(),
        expires_at_unix_ms,
        candidates: candidates_to_proto(candidates),
    };
    validate_invite(&invite)?;
    Ok(invite)
}

fn parse_proto_candidate(candidate: &ProtoCandidate) -> CoreResult<EndpointCandidate> {
    let address: SocketAddr = candidate.address.parse().map_err(|_| {
        CoreError::InvalidInvitation(format!("invalid candidate address {}", candidate.address))
    })?;
    if address.port() == 0 {
        return Err(CoreError::InvalidInvitation(
            "candidate port must be nonzero".to_owned(),
        ));
    }
    let kind = match ProtoCandidateKind::try_from(candidate.kind) {
        Ok(ProtoCandidateKind::GlobalIpv6) => CandidateKind::GlobalIpv6,
        Ok(ProtoCandidateKind::PublicIpv4) => CandidateKind::PublicIpv4,
        Ok(ProtoCandidateKind::RouterMapping) => CandidateKind::RouterMapping,
        Ok(ProtoCandidateKind::Manual) => CandidateKind::Manual,
        Ok(ProtoCandidateKind::PeerObserved) => CandidateKind::PeerObserved,
        Ok(ProtoCandidateKind::LocalNetwork) => CandidateKind::LocalNetwork,
        _ => {
            return Err(CoreError::InvalidInvitation(
                "unknown candidate kind".to_owned(),
            ));
        }
    };
    let priority = u16::try_from(candidate.priority).unwrap_or(kind.priority());
    Ok(EndpointCandidate {
        kind,
        address,
        priority,
        observed_at_unix_ms: now_unix_ms(),
        expires_at_unix_ms: kind.default_expiry(now_unix_ms()),
        source_node_id: None,
        reachability: crate::CandidateReachability::Unknown,
    })
}

fn proto_kind(kind: CandidateKind) -> ProtoCandidateKind {
    match kind {
        CandidateKind::GlobalIpv6 => ProtoCandidateKind::GlobalIpv6,
        CandidateKind::PublicIpv4 => ProtoCandidateKind::PublicIpv4,
        CandidateKind::RouterMapping => ProtoCandidateKind::RouterMapping,
        CandidateKind::Manual => ProtoCandidateKind::Manual,
        CandidateKind::PeerObserved => ProtoCandidateKind::PeerObserved,
        CandidateKind::LocalNetwork => ProtoCandidateKind::LocalNetwork,
    }
}
