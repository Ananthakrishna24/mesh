mod candidates;
mod endpoint;
mod error;
mod frame;
mod handshake;
mod identity;
mod tls;

pub use candidates::collect_local_candidates;
pub use endpoint::{IncomingPeer, MeshEndpoint, PeerConnection};
pub use error::{NetError, NetResult};
pub use frame::{read_envelope, write_envelope};
pub use handshake::{
    EnrollmentHello, WelcomePayload, complete_inviter_handshake, perform_joiner_handshake,
};
pub use identity::generate_node_certificate;

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::time::Duration;

    use mesh_core::{EnrollmentId, LocalIdentity, MeshId, now_unix_ms};

    use super::*;

    #[tokio::test]
    async fn localhost_hello_welcome() {
        let inviter_cert = generate_node_certificate().expect("inviter cert");
        let joiner_cert = generate_node_certificate().expect("joiner cert");
        let mesh_id = MeshId::new();

        let inviter_identity = LocalIdentity {
            node_id: inviter_cert.node_id,
            mesh_id,
            display_name: "Inviter".to_owned(),
            certificate_der: inviter_cert.certificate_der,
            private_key_der: inviter_cert.private_key_der,
            created_at_unix_ms: now_unix_ms(),
        };
        let joiner_identity = LocalIdentity {
            node_id: joiner_cert.node_id,
            mesh_id,
            display_name: "Joiner".to_owned(),
            certificate_der: joiner_cert.certificate_der,
            private_key_der: joiner_cert.private_key_der,
            created_at_unix_ms: now_unix_ms(),
        };

        let inviter_endpoint = MeshEndpoint::bind(
            inviter_identity.clone(),
            SocketAddr::from(([127, 0, 0, 1], 0)),
        )
        .expect("bind inviter");
        let listen = inviter_endpoint.listen_addr();
        let inviter_candidates = collect_local_candidates(listen);

        let accept = tokio::spawn({
            let inviter_endpoint = inviter_endpoint.clone();
            let inviter_identity = inviter_identity.clone();
            let inviter_candidates = inviter_candidates.clone();
            async move {
                let incoming = inviter_endpoint.accept().await.expect("accept");
                let peer = complete_inviter_handshake(
                    &incoming.connection,
                    &inviter_identity,
                    &inviter_candidates,
                    &[],
                    incoming.peer_certificate_der,
                    |_hello, _peer| Ok(()),
                )
                .await
                .expect("inviter handshake");
                tokio::time::sleep(Duration::from_millis(200)).await;
                peer
            }
        });

        tokio::time::sleep(Duration::from_millis(50)).await;

        let mut joiner_endpoint =
            MeshEndpoint::bind(joiner_identity.clone(), SocketAddr::from(([127, 0, 0, 1], 0)))
                .expect("bind joiner");
        let joiner_candidates = collect_local_candidates(joiner_endpoint.listen_addr());
        let connected = joiner_endpoint
            .connect(listen, inviter_identity.node_id)
            .await
            .expect("connect");

        let welcome = perform_joiner_handshake(
            &connected.connection,
            &joiner_identity,
            &joiner_candidates,
            Some(EnrollmentId::new()),
            Some([9u8; 32]),
            inviter_identity.node_id,
        )
        .await
        .expect("joiner handshake");

        let peer = accept.await.expect("accept task");
        assert_eq!(welcome.responder.node_id, inviter_identity.node_id);
        assert_eq!(peer.node_id, joiner_identity.node_id);
    }
}
