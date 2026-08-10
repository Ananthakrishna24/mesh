use std::time::Duration;

use bytes::Bytes;
use mesh_core::invite::{candidates_from_proto, candidates_to_proto};
use mesh_core::protocol::proto::{
    ControlEnvelope, ErrorCode, ErrorMessage, Hello, PeerRecord as ProtoPeer, Welcome,
    control_envelope::Body,
};
use mesh_core::{
    EndpointCandidate, EnrollmentId, LocalIdentity, MeshId, NodeId, PeerRecord, PROTOCOL_MAJOR,
    PROTOCOL_MINOR, PROTOCOL_MINOR_MIN, capability_digest, random_message_id,
};
use quinn::{Connection, RecvStream, SendStream};
use tokio::time::timeout;

use crate::frame::{read_envelope, write_envelope};
use crate::{NetError, NetResult};

#[derive(Debug, Clone)]
pub struct EnrollmentHello {
    pub mesh_id: MeshId,
    pub display_name: String,
    pub enrollment_id: Option<EnrollmentId>,
    pub enrollment_secret: Option<Vec<u8>>,
    pub candidates: Vec<EndpointCandidate>,
    pub sender_node_id: NodeId,
    pub message_id: Bytes,
}

#[derive(Debug, Clone)]
pub struct WelcomePayload {
    pub selected_protocol_minor: u32,
    pub responder: PeerRecord,
    pub known_peers: Vec<PeerRecord>,
}

pub async fn open_control_stream(connection: &Connection) -> NetResult<(SendStream, RecvStream)> {
    Ok(connection.open_bi().await?)
}

pub async fn accept_control_stream(connection: &Connection) -> NetResult<(SendStream, RecvStream)> {
    let (send, recv) = timeout(Duration::from_secs(10), connection.accept_bi())
        .await
        .map_err(|_| NetError::Timeout)??;
    Ok((send, recv))
}

pub fn build_hello(
    identity: &LocalIdentity,
    candidates: &[EndpointCandidate],
    enrollment_id: Option<EnrollmentId>,
    enrollment_secret: Option<[u8; 32]>,
) -> ControlEnvelope {
    let hello = Hello {
        mesh_id: identity.mesh_id.to_vec().into(),
        minimum_protocol_minor: PROTOCOL_MINOR_MIN,
        display_name: identity.display_name.clone(),
        enrollment_id: enrollment_id.map(|id| id.to_vec().into()),
        enrollment_secret: enrollment_secret.map(|secret| secret.to_vec().into()),
        capability_digest: capability_digest(&[
            identity.node_id.as_bytes().as_slice(),
            identity.display_name.as_bytes(),
        ]),
        candidates: candidates_to_proto(candidates),
    };

    ControlEnvelope {
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor: PROTOCOL_MINOR,
        message_id: random_message_id(),
        sender_node_id: identity.node_id.to_vec().into(),
        in_reply_to: None,
        body: Some(Body::Hello(hello)),
    }
}

pub fn build_welcome(
    identity: &LocalIdentity,
    local_candidates: &[EndpointCandidate],
    known_peers: &[PeerRecord],
    in_reply_to: Bytes,
) -> ControlEnvelope {
    let responder = PeerRecord {
        node_id: identity.node_id,
        display_name: identity.display_name.clone(),
        certificate_der: identity.certificate_der.clone(),
        candidates: local_candidates.to_vec(),
    };
    let welcome = Welcome {
        selected_protocol_minor: PROTOCOL_MINOR,
        responder: Some(peer_to_proto(&responder)),
        known_peers: known_peers.iter().map(peer_to_proto).collect(),
    };
    ControlEnvelope {
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor: PROTOCOL_MINOR,
        message_id: random_message_id(),
        sender_node_id: identity.node_id.to_vec().into(),
        in_reply_to: Some(in_reply_to),
        body: Some(Body::Welcome(welcome)),
    }
}

pub fn build_error(
    identity: &LocalIdentity,
    code: ErrorCode,
    summary: impl Into<String>,
    detail: impl Into<String>,
    related_message_id: Option<Bytes>,
) -> ControlEnvelope {
    ControlEnvelope {
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor: PROTOCOL_MINOR,
        message_id: random_message_id(),
        sender_node_id: identity.node_id.to_vec().into(),
        in_reply_to: related_message_id.clone(),
        body: Some(Body::Error(ErrorMessage {
            code: code as i32,
            related_message_id,
            retriable: false,
            summary: summary.into(),
            detail: detail.into(),
        })),
    }
}

pub async fn perform_joiner_handshake(
    connection: &Connection,
    identity: &LocalIdentity,
    local_candidates: &[EndpointCandidate],
    enrollment_id: Option<EnrollmentId>,
    enrollment_secret: Option<[u8; 32]>,
    expected_inviter: NodeId,
) -> NetResult<WelcomePayload> {
    let (mut send, mut recv) = open_control_stream(connection).await?;
    let hello = build_hello(identity, local_candidates, enrollment_id, enrollment_secret);
    write_envelope(&mut send, &hello).await?;

    let envelope = timeout(Duration::from_secs(10), read_envelope(&mut recv))
        .await
        .map_err(|_| NetError::Timeout)??;
    let sender = NodeId::from_slice(&envelope.sender_node_id)?;
    if sender != expected_inviter {
        return Err(NetError::Protocol(
            "envelope sender does not match expected inviter".to_owned(),
        ));
    }
    if envelope.protocol_major != PROTOCOL_MAJOR {
        return Err(NetError::Protocol("unsupported protocol major".to_owned()));
    }

    match envelope.body {
        Some(Body::Welcome(welcome)) => {
            let payload = decode_welcome(welcome)?;
            if payload.responder.node_id != expected_inviter {
                return Err(NetError::Protocol(
                    "welcome responder node id mismatch".to_owned(),
                ));
            }
            Ok(payload)
        }
        Some(Body::Error(error)) => Err(NetError::Protocol(format!(
            "handshake rejected: {}",
            error.summary
        ))),
        _ => Err(NetError::Protocol(
            "expected WELCOME during joiner handshake".to_owned(),
        )),
    }
}

pub async fn complete_inviter_handshake(
    connection: &Connection,
    identity: &LocalIdentity,
    local_candidates: &[EndpointCandidate],
    known_peers: &[PeerRecord],
    peer_certificate_der: Vec<u8>,
    mut accept_enrollment: impl FnMut(EnrollmentHello, PeerRecord) -> Result<(), (ErrorCode, String)>,
) -> NetResult<PeerRecord> {
    let (mut send, mut recv) = accept_control_stream(connection).await?;
    let envelope = timeout(Duration::from_secs(10), read_envelope(&mut recv))
        .await
        .map_err(|_| NetError::Timeout)??;

    if envelope.protocol_major != PROTOCOL_MAJOR {
        let error = build_error(
            identity,
            ErrorCode::UnsupportedProtocol,
            "Unsupported protocol version.",
            "protocol major mismatch",
            Some(envelope.message_id.clone()),
        );
        let _ = write_envelope(&mut send, &error).await;
        return Err(NetError::Protocol("unsupported protocol major".to_owned()));
    }

    let sender = NodeId::from_slice(&envelope.sender_node_id)?;
    let tls_node_id = NodeId::from_certificate_der(&peer_certificate_der);
    if sender != tls_node_id {
        let error = build_error(
            identity,
            ErrorCode::IdentityMismatch,
            "Identity mismatch.",
            "HELLO sender does not match TLS certificate",
            Some(envelope.message_id.clone()),
        );
        let _ = write_envelope(&mut send, &error).await;
        return Err(NetError::Protocol("sender/tls mismatch".to_owned()));
    }

    let Some(Body::Hello(hello)) = envelope.body else {
        let error = build_error(
            identity,
            ErrorCode::InvalidState,
            "Expected HELLO.",
            "first control message was not HELLO",
            Some(envelope.message_id.clone()),
        );
        let _ = write_envelope(&mut send, &error).await;
        return Err(NetError::Protocol("expected HELLO".to_owned()));
    };

    let parsed = parse_hello(sender, &envelope.message_id, hello)?;
    if parsed.mesh_id != identity.mesh_id {
        let error = build_error(
            identity,
            ErrorCode::MeshMismatch,
            "Mesh ID mismatch.",
            "mesh id mismatch",
            Some(envelope.message_id.clone()),
        );
        let _ = write_envelope(&mut send, &error).await;
        return Err(NetError::Protocol("mesh mismatch".to_owned()));
    }

    let peer = PeerRecord {
        node_id: sender,
        display_name: parsed.display_name.clone(),
        certificate_der: peer_certificate_der,
        candidates: parsed.candidates.clone(),
    };

    if parsed.enrollment_id.is_some() {
        if let Err((code, detail)) = accept_enrollment(parsed, peer.clone()) {
            let error = build_error(
                identity,
                code,
                summary_for(code),
                detail,
                Some(envelope.message_id.clone()),
            );
            let _ = write_envelope(&mut send, &error).await;
            return Err(NetError::Protocol("enrollment rejected".to_owned()));
        }
    } else if !known_peers.iter().any(|known| known.node_id == sender) {
        let error = build_error(
            identity,
            ErrorCode::UnknownPeer,
            "Unknown peer.",
            "reconnect from node not in peer store",
            Some(envelope.message_id.clone()),
        );
        let _ = write_envelope(&mut send, &error).await;
        return Err(NetError::Protocol("unknown peer reconnect".to_owned()));
    }

    let welcome = build_welcome(
        identity,
        local_candidates,
        known_peers,
        envelope.message_id.clone(),
    );
    write_envelope(&mut send, &welcome).await?;
    Ok(peer)
}

pub fn decode_welcome(welcome: Welcome) -> NetResult<WelcomePayload> {
    let responder = welcome
        .responder
        .ok_or_else(|| NetError::Protocol("welcome missing responder".to_owned()))?;
    Ok(WelcomePayload {
        selected_protocol_minor: welcome.selected_protocol_minor,
        responder: peer_from_proto(responder)?,
        known_peers: welcome
            .known_peers
            .into_iter()
            .map(peer_from_proto)
            .collect::<NetResult<Vec<_>>>()?,
    })
}

fn parse_hello(sender: NodeId, message_id: &Bytes, hello: Hello) -> NetResult<EnrollmentHello> {
    if hello.display_name.trim().is_empty() || hello.display_name.len() > 128 {
        return Err(NetError::Protocol("invalid display name".to_owned()));
    }
    if hello.minimum_protocol_minor > PROTOCOL_MINOR {
        return Err(NetError::Protocol(
            "peer requires newer protocol minor".to_owned(),
        ));
    }
    let enrollment_id = hello
        .enrollment_id
        .as_ref()
        .map(|bytes| EnrollmentId::from_slice(bytes))
        .transpose()?;
    let enrollment_secret = hello.enrollment_secret.map(|bytes| bytes.to_vec());
    if enrollment_id.is_some()
        != enrollment_secret
            .as_ref()
            .is_some_and(|secret| secret.len() == 32)
    {
        return Err(NetError::Protocol(
            "enrollment id/secret pairing is invalid".to_owned(),
        ));
    }
    Ok(EnrollmentHello {
        mesh_id: MeshId::from_slice(&hello.mesh_id)?,
        display_name: hello.display_name,
        enrollment_id,
        enrollment_secret,
        candidates: candidates_from_proto(&hello.candidates)?,
        sender_node_id: sender,
        message_id: message_id.clone(),
    })
}

fn peer_to_proto(peer: &PeerRecord) -> ProtoPeer {
    ProtoPeer {
        node_id: peer.node_id.to_vec().into(),
        display_name: peer.display_name.clone(),
        certificate_der: peer.certificate_der.clone().into(),
        candidates: candidates_to_proto(&peer.candidates),
    }
}

fn peer_from_proto(peer: ProtoPeer) -> NetResult<PeerRecord> {
    let node_id = NodeId::from_slice(&peer.node_id)?;
    if peer.certificate_der.is_empty() {
        return Err(NetError::Protocol(
            "peer record missing certificate".to_owned(),
        ));
    }
    let derived = NodeId::from_certificate_der(&peer.certificate_der);
    if derived != node_id {
        return Err(NetError::Protocol(
            "peer record node id does not match certificate".to_owned(),
        ));
    }
    if peer.display_name.trim().is_empty() || peer.display_name.len() > 128 {
        return Err(NetError::Protocol("invalid peer display name".to_owned()));
    }
    Ok(PeerRecord {
        node_id,
        display_name: peer.display_name,
        certificate_der: peer.certificate_der.to_vec(),
        candidates: candidates_from_proto(&peer.candidates)?,
    })
}

fn summary_for(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::InviteInvalid => "This invitation is invalid.",
        ErrorCode::InviteExpired => "This invitation has expired.",
        ErrorCode::InviteAlreadyUsed => "This invitation was already used by another PC.",
        ErrorCode::UnknownPeer => "This PC is not recognized.",
        ErrorCode::MeshMismatch => "Mesh ID mismatch.",
        ErrorCode::IdentityMismatch => "Identity mismatch.",
        ErrorCode::UnsupportedProtocol => "Unsupported protocol version.",
        _ => "Enrollment failed.",
    }
}
