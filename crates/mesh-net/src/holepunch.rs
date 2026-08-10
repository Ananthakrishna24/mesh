use std::net::SocketAddr;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use mesh_core::protocol::proto::{
    ControlEnvelope, IntroductionOffer, IntroductionReady, PeerObserve, control_envelope::Body,
};
use mesh_core::{
    CandidateKind, EndpointCandidate, LocalIdentity, NodeId, PROTOCOL_MAJOR, PROTOCOL_MINOR,
    now_unix_ms, random_message_id,
};
use rand::RngCore;
use tokio::time::{Instant, sleep};

use crate::frame::write_envelope;
use crate::{NetError, NetResult};

pub const HOLE_PUNCH_WINDOW: Duration = Duration::from_millis(800);
pub const HOLE_PUNCH_PROBE_INTERVAL: Duration = Duration::from_millis(40);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntroductionAttempt {
    pub attempt_id: [u8; 16],
    pub target_node_id: NodeId,
    pub peer_observed: SocketAddr,
    pub start_at_unix_ms: i64,
}

pub fn new_attempt_id() -> [u8; 16] {
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    bytes
}

pub fn build_introduction_offer(
    identity: &LocalIdentity,
    target_node_id: NodeId,
    observed: SocketAddr,
    attempt_id: [u8; 16],
    start_at_unix_ms: i64,
) -> ControlEnvelope {
    ControlEnvelope {
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor: PROTOCOL_MINOR,
        message_id: random_message_id(),
        sender_node_id: identity.node_id.to_vec().into(),
        in_reply_to: None,
        body: Some(Body::IntroductionOffer(IntroductionOffer {
            target_node_id: target_node_id.to_vec().into(),
            attempt_id: Bytes::copy_from_slice(&attempt_id),
            start_at_unix_ms,
            observed_address: observed.to_string(),
        })),
    }
}

pub fn build_introduction_ready(
    identity: &LocalIdentity,
    attempt_id: [u8; 16],
    peer_node_id: NodeId,
    peer_observed: SocketAddr,
    self_observed: SocketAddr,
    start_at_unix_ms: i64,
) -> ControlEnvelope {
    ControlEnvelope {
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor: PROTOCOL_MINOR,
        message_id: random_message_id(),
        sender_node_id: identity.node_id.to_vec().into(),
        in_reply_to: None,
        body: Some(Body::IntroductionReady(IntroductionReady {
            attempt_id: Bytes::copy_from_slice(&attempt_id),
            peer_node_id: peer_node_id.to_vec().into(),
            peer_observed_address: peer_observed.to_string(),
            self_observed_address: self_observed.to_string(),
            start_at_unix_ms,
        })),
    }
}

pub fn build_peer_observe(
    identity: &LocalIdentity,
    observed_node_id: NodeId,
    address: SocketAddr,
) -> ControlEnvelope {
    ControlEnvelope {
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor: PROTOCOL_MINOR,
        message_id: random_message_id(),
        sender_node_id: identity.node_id.to_vec().into(),
        in_reply_to: None,
        body: Some(Body::PeerObserve(PeerObserve {
            observed_node_id: observed_node_id.to_vec().into(),
            address: address.to_string(),
            observed_at_unix_ms: now_unix_ms(),
        })),
    }
}

pub fn peer_observed_candidate(address: SocketAddr, source_node_id: NodeId) -> EndpointCandidate {
    EndpointCandidate::new(CandidateKind::PeerObserved, address).with_source(source_node_id)
}

pub fn parse_socket_addr(value: &str) -> NetResult<SocketAddr> {
    value
        .parse()
        .map_err(|_| NetError::Protocol(format!("invalid socket address {value}")))
}

pub fn start_at_after(delay: Duration) -> i64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0);
    now + delay.as_millis() as i64
}

pub async fn wait_until_unix_ms(start_at_unix_ms: i64) {
    let now = now_unix_ms();
    if start_at_unix_ms > now {
        sleep(Duration::from_millis((start_at_unix_ms - now) as u64)).await;
    }
}

/// Best-effort UDP probes to open NAT bindings before a Quinn dial.
/// Uses a short-lived socket so mesh traffic stays on the Quinn socket.
pub async fn send_udp_probes(targets: &[SocketAddr], window: Duration) {
    if targets.is_empty() {
        return;
    }
    let socket = match tokio::net::UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], 0))).await {
        Ok(socket) => socket,
        Err(_) => return,
    };
    let payload = b"mesh-hp";
    let deadline = Instant::now() + window;
    while Instant::now() < deadline {
        for target in targets {
            let _ = socket.send_to(payload, *target).await;
        }
        sleep(HOLE_PUNCH_PROBE_INTERVAL).await;
    }
}

pub async fn write_control(
    send: &mut quinn::SendStream,
    envelope: &ControlEnvelope,
) -> NetResult<()> {
    write_envelope(send, envelope).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesh_core::MeshId;

    #[test]
    fn attempt_id_is_nonzero() {
        assert_ne!(new_attempt_id(), [0u8; 16]);
    }

    #[test]
    fn offer_roundtrip_fields() {
        let identity = LocalIdentity {
            node_id: NodeId::from_bytes([1; 32]),
            mesh_id: MeshId::from_bytes([2; 16]),
            display_name: "A".to_owned(),
            certificate_der: vec![1, 2, 3],
            private_key_der: vec![4, 5, 6],
            created_at_unix_ms: 1,
        };
        let target = NodeId::from_bytes([9; 32]);
        let offer = build_introduction_offer(
            &identity,
            target,
            "1.2.3.4:5".parse().unwrap(),
            [7; 16],
            100,
        );
        match offer.body {
            Some(Body::IntroductionOffer(body)) => {
                assert_eq!(body.target_node_id.as_ref(), target.as_bytes());
                assert_eq!(body.attempt_id.as_ref(), &[7; 16]);
                assert_eq!(body.observed_address, "1.2.3.4:5");
            }
            other => panic!("unexpected body {other:?}"),
        }
    }
}
