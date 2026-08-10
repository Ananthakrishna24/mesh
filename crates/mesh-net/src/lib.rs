mod benchmark;
mod candidates;
mod endpoint;
mod error;
mod frame;
mod handshake;
mod identity;
mod session;
mod tls;

pub use benchmark::{
    BandwidthBenchmarkOutcome, DelayBenchmarkOutcome, build_capability_envelope,
    capability_from_proto, capability_to_proto, default_bandwidth_payload, run_bandwidth_receive,
    run_bandwidth_send, run_delay_benchmark, respond_bandwidth_receive, respond_bandwidth_send,
    respond_delay_benchmark, summarize_delay_samples,
};
pub use candidates::collect_local_candidates;
pub use endpoint::{IncomingPeer, MeshEndpoint, PeerConnection};
pub use error::{NetError, NetResult};
pub use frame::{read_envelope, write_envelope};
pub use handshake::{
    EnrollmentHello, WelcomePayload, complete_inviter_handshake, perform_joiner_handshake,
};
pub use identity::generate_node_certificate;
pub use session::{SessionEvent, run_connected_session};

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::time::Duration;

    use mesh_core::{
        CapabilityReport, ComputeProxy, EnrollmentId, LocalIdentity, MeshId, now_unix_ms,
    };
    use tokio::sync::mpsc;

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
                let (peer, _send, _recv) = complete_inviter_handshake(
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

        let (welcome, _send, _recv) = perform_joiner_handshake(
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn localhost_capability_and_bandwidth() {
        let left_cert = generate_node_certificate().expect("left cert");
        let right_cert = generate_node_certificate().expect("right cert");
        let mesh_id = MeshId::new();
        let left_identity = LocalIdentity {
            node_id: left_cert.node_id,
            mesh_id,
            display_name: "Left".to_owned(),
            certificate_der: left_cert.certificate_der,
            private_key_der: left_cert.private_key_der,
            created_at_unix_ms: now_unix_ms(),
        };
        let right_identity = LocalIdentity {
            node_id: right_cert.node_id,
            mesh_id,
            display_name: "Right".to_owned(),
            certificate_der: right_cert.certificate_der,
            private_key_der: right_cert.private_key_der,
            created_at_unix_ms: now_unix_ms(),
        };

        let left_endpoint =
            MeshEndpoint::bind(left_identity.clone(), SocketAddr::from(([127, 0, 0, 1], 0)))
                .expect("bind left");
        let listen = left_endpoint.listen_addr();
        let left_candidates = collect_local_candidates(listen);
        let report = CapabilityReport {
            collected_at_unix_ms: now_unix_ms(),
            os: "test".into(),
            arch: "x64".into(),
            cpu_model: "test-cpu".into(),
            cpu_logical_cores: 4,
            memory_total_bytes: 8 << 30,
            memory_available_bytes: 4 << 30,
            disk_total_bytes: 100 << 30,
            disk_available_bytes: 50 << 30,
            gpus: Vec::new(),
            compute: ComputeProxy {
                cpu_fp32_gflops: 1.0,
                measured_at_unix_ms: now_unix_ms(),
            },
            status: "ok".into(),
        };

        let left_report = report.clone();
        let right_report = report.clone();
        let left_identity_task = left_identity.clone();
        let accept = tokio::spawn(async move {
            let incoming = left_endpoint.accept().await.expect("accept");
            let (peer, send, recv) = complete_inviter_handshake(
                &incoming.connection,
                &left_identity_task,
                &left_candidates,
                &[],
                incoming.peer_certificate_der,
                |_hello, _peer| Ok(()),
            )
            .await
            .expect("inviter handshake");
            let (tx, mut rx) = mpsc::channel(16);
            let peer_id = peer.node_id;
            let session = tokio::spawn(async move {
                run_connected_session(
                    left_identity_task,
                    peer_id,
                    incoming.connection,
                    send,
                    recv,
                    left_report,
                    tx,
                )
                .await;
            });
            let mut saw_delay = false;
            let mut saw_bandwidth = false;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
            while tokio::time::Instant::now() < deadline {
                match tokio::time::timeout(Duration::from_millis(500), rx.recv()).await {
                    Ok(Some(SessionEvent::Link { measurement, .. })) => {
                        saw_delay |= measurement.delay.is_some();
                        saw_bandwidth |= measurement.to_peer_bandwidth.is_some()
                            || measurement.from_peer_bandwidth.is_some();
                        if saw_delay && saw_bandwidth {
                            break;
                        }
                    }
                    Ok(Some(SessionEvent::Failed { message, .. })) => {
                        panic!("session failed: {message}");
                    }
                    Ok(Some(_)) | Err(_) => {}
                    Ok(None) => break,
                }
            }
            session.abort();
            (saw_delay, saw_bandwidth)
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        let mut right_endpoint =
            MeshEndpoint::bind(right_identity.clone(), SocketAddr::from(([127, 0, 0, 1], 0)))
                .expect("bind right");
        let right_candidates = collect_local_candidates(right_endpoint.listen_addr());
        let connected = right_endpoint
            .connect(listen, left_identity.node_id)
            .await
            .expect("connect");
        let (_welcome, send, recv) = perform_joiner_handshake(
            &connected.connection,
            &right_identity,
            &right_candidates,
            Some(EnrollmentId::new()),
            Some([7u8; 32]),
            left_identity.node_id,
        )
        .await
        .expect("joiner handshake");
        let (tx, mut rx) = mpsc::channel(16);
        let right_identity_task = right_identity.clone();
        let peer_id = left_identity.node_id;
        let session = tokio::spawn(async move {
            run_connected_session(
                right_identity_task,
                peer_id,
                connected.connection,
                send,
                recv,
                right_report,
                tx,
            )
            .await;
        });
        let mut saw_delay = false;
        let mut saw_bandwidth = false;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(500), rx.recv()).await {
                Ok(Some(SessionEvent::Link { measurement, .. })) => {
                    saw_delay |= measurement.delay.is_some();
                    saw_bandwidth |= measurement.to_peer_bandwidth.is_some()
                        || measurement.from_peer_bandwidth.is_some();
                    if saw_delay && saw_bandwidth {
                        break;
                    }
                }
                Ok(Some(SessionEvent::Failed { message, .. })) => {
                    panic!("right session failed: {message}");
                }
                Ok(Some(_)) | Err(_) => {}
                Ok(None) => break,
            }
        }
        session.abort();
        let (left_delay, left_bw) = accept.await.expect("accept task");
        assert!(saw_delay || left_delay, "expected delay measurement");
        assert!(saw_bandwidth || left_bw, "expected bandwidth measurement");
    }
}
