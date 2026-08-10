use mesh_core::protocol::proto::{
    BenchmarkDirection, BenchmarkKind, BenchmarkRequest, ControlEnvelope, IntroductionOffer,
    IntroductionReady, PeerObserve, PeerRecord as ProtoPeer, PeerUpdate, control_envelope::Body,
};
use mesh_core::{
    BandwidthMeasurement, CapabilityReport, DelayMeasurement, EndpointCandidate, LinkMeasurement,
    LocalIdentity, NodeId, PeerRecord, now_unix_ms, random_message_id, PROTOCOL_MAJOR,
    PROTOCOL_MINOR,
};
use mesh_core::invite::{candidates_from_proto, candidates_to_proto};
use quinn::{Connection, RecvStream, SendStream};
use tokio::sync::mpsc;
use tracing::warn;

use crate::benchmark::{
    build_capability_envelope, capability_from_proto, default_bandwidth_payload,
    respond_bandwidth_receive, respond_bandwidth_send, respond_delay_benchmark,
    run_bandwidth_receive, run_bandwidth_send, run_delay_benchmark,
};
use crate::frame::{read_envelope, write_envelope};
use crate::holepunch::{
    build_introduction_offer, build_introduction_ready, parse_socket_addr, peer_observed_candidate,
};
use crate::{NetError, NetResult};

#[derive(Debug, Clone)]
pub enum SessionEvent {
    Capability {
        peer_node_id: NodeId,
        report: CapabilityReport,
    },
    Link {
        peer_node_id: NodeId,
        measurement: LinkMeasurement,
    },
    PeerUpdate {
        from_peer: NodeId,
        peers: Vec<PeerRecord>,
    },
    IntroductionOffer {
        from_peer: NodeId,
        target_node_id: NodeId,
        attempt_id: [u8; 16],
        start_at_unix_ms: i64,
        observed_address: std::net::SocketAddr,
    },
    IntroductionReady {
        from_peer: NodeId,
        attempt_id: [u8; 16],
        peer_node_id: NodeId,
        peer_observed: std::net::SocketAddr,
        self_observed: std::net::SocketAddr,
        start_at_unix_ms: i64,
    },
    PeerObserve {
        from_peer: NodeId,
        observed_node_id: NodeId,
        address: std::net::SocketAddr,
        observed_at_unix_ms: i64,
    },
    Failed {
        peer_node_id: NodeId,
        message: String,
    },
}

#[derive(Debug, Clone)]
pub enum SessionCommand {
    SendPeerUpdate { peers: Vec<PeerRecord> },
    SendIntroductionOffer {
        target_node_id: NodeId,
        attempt_id: [u8; 16],
        start_at_unix_ms: i64,
        observed_address: std::net::SocketAddr,
    },
    SendIntroductionReady {
        attempt_id: [u8; 16],
        peer_node_id: NodeId,
        peer_observed: std::net::SocketAddr,
        self_observed: std::net::SocketAddr,
        start_at_unix_ms: i64,
    },
}

pub async fn run_connected_session(
    identity: LocalIdentity,
    peer_node_id: NodeId,
    connection: Connection,
    mut send: SendStream,
    mut recv: RecvStream,
    local_report: CapabilityReport,
    events: mpsc::Sender<SessionEvent>,
    mut commands: mpsc::Receiver<SessionCommand>,
) {
    if let Err(error) = write_envelope(
        &mut send,
        &build_capability_envelope(&identity, &local_report),
    )
    .await
    {
        let _ = events
            .send(SessionEvent::Failed {
                peer_node_id,
                message: error.to_string(),
            })
            .await;
        return;
    }

    let mut link = LinkMeasurement::empty(peer_node_id);
    if let Err(error) = drive_session(
        &identity,
        peer_node_id,
        &connection,
        &mut send,
        &mut recv,
        &mut link,
        &events,
        &mut commands,
    )
    .await
    {
        let _ = events
            .send(SessionEvent::Failed {
                peer_node_id,
                message: error.to_string(),
            })
            .await;
    }
}

async fn drive_session(
    identity: &LocalIdentity,
    peer_node_id: NodeId,
    connection: &Connection,
    send: &mut SendStream,
    recv: &mut RecvStream,
    link: &mut LinkMeasurement,
    events: &mpsc::Sender<SessionEvent>,
    commands: &mut mpsc::Receiver<SessionCommand>,
) -> NetResult<()> {
    // Both peers emit capability first. Drain that report before any benchmark
    // so initiator accept/result reads stay aligned with the peer.
    let first = read_envelope(recv).await?;
    handle_peer_message(
        identity,
        peer_node_id,
        connection,
        send,
        recv,
        link,
        events,
        first,
    )
    .await?;

    let initiate = identity.node_id < peer_node_id;
    if initiate {
        match run_delay_benchmark(identity, send, recv).await {
            Ok(outcome) => {
                link.delay = Some(outcome.measurement.clone());
                publish_link(events, peer_node_id, link).await;
            }
            Err(error) => warn!(%error, "delay benchmark failed"),
        }

        match run_bandwidth_send(
            identity,
            connection,
            send,
            recv,
            default_bandwidth_payload(),
        )
        .await
        {
            Ok(outcome) => {
                link.to_peer_bandwidth = Some(BandwidthMeasurement {
                    bandwidth_bps: outcome.bandwidth_bps,
                    payload_bytes: outcome.payload_bytes,
                    transfer_ms: outcome.transfer_ms,
                    measured_at_unix_ms: outcome.measured_at_unix_ms,
                });
                publish_link(events, peer_node_id, link).await;
            }
            Err(error) => warn!(%error, "outbound bandwidth benchmark failed"),
        }

        match run_bandwidth_receive(
            identity,
            connection,
            send,
            recv,
            default_bandwidth_payload(),
        )
        .await
        {
            Ok(outcome) => {
                link.from_peer_bandwidth = Some(BandwidthMeasurement {
                    bandwidth_bps: outcome.bandwidth_bps,
                    payload_bytes: outcome.payload_bytes,
                    transfer_ms: outcome.transfer_ms,
                    measured_at_unix_ms: outcome.measured_at_unix_ms,
                });
                publish_link(events, peer_node_id, link).await;
            }
            Err(error) => warn!(%error, "inbound bandwidth benchmark failed"),
        }
    }

    loop {
        tokio::select! {
            command = commands.recv() => {
                match command {
                    Some(command) => {
                        handle_session_command(identity, send, command).await?;
                    }
                    None => return Ok(()),
                }
            }
            envelope = read_envelope(recv) => {
                let envelope = match envelope {
                    Ok(envelope) => envelope,
                    Err(NetError::Closed) | Err(NetError::Read(_)) => return Ok(()),
                    Err(error) => return Err(error),
                };
                handle_peer_message(
                    identity,
                    peer_node_id,
                    connection,
                    send,
                    recv,
                    link,
                    events,
                    envelope,
                )
                .await?;
            }
        }
    }
}

async fn handle_session_command(
    identity: &LocalIdentity,
    send: &mut SendStream,
    command: SessionCommand,
) -> NetResult<()> {
    match command {
        SessionCommand::SendPeerUpdate { peers } => {
            let envelope = build_peer_update_envelope(identity, &peers);
            write_envelope(send, &envelope).await
        }
        SessionCommand::SendIntroductionOffer {
            target_node_id,
            attempt_id,
            start_at_unix_ms,
            observed_address,
        } => {
            let envelope = build_introduction_offer(
                identity,
                target_node_id,
                observed_address,
                attempt_id,
                start_at_unix_ms,
            );
            write_envelope(send, &envelope).await
        }
        SessionCommand::SendIntroductionReady {
            attempt_id,
            peer_node_id,
            peer_observed,
            self_observed,
            start_at_unix_ms,
        } => {
            let envelope = build_introduction_ready(
                identity,
                attempt_id,
                peer_node_id,
                peer_observed,
                self_observed,
                start_at_unix_ms,
            );
            write_envelope(send, &envelope).await
        }
    }
}

async fn handle_peer_message(
    identity: &LocalIdentity,
    peer_node_id: NodeId,
    connection: &Connection,
    send: &mut SendStream,
    recv: &mut RecvStream,
    link: &mut LinkMeasurement,
    events: &mpsc::Sender<SessionEvent>,
    envelope: ControlEnvelope,
) -> NetResult<()> {
    match envelope.body {
        Some(Body::CapabilityReport(report)) => {
            let report = capability_from_proto(report)?;
            let _ = events
                .send(SessionEvent::Capability {
                    peer_node_id,
                    report,
                })
                .await;
            Ok(())
        }
        Some(Body::BenchmarkRequest(request)) => {
            handle_benchmark_request(
                identity,
                peer_node_id,
                connection,
                send,
                recv,
                link,
                events,
                request,
            )
            .await
        }
        Some(Body::BenchmarkResult(result)) => {
            if result.kind == BenchmarkKind::Delay as i32 {
                link.delay = Some(DelayMeasurement {
                    one_way_delay_ms: result.one_way_delay_ms,
                    rtt_ms: result.rtt_ms,
                    rtt_p95_ms: result.rtt_p95_ms,
                    stability_score: result.stability_score.min(u32::from(u8::MAX)) as u8,
                    sample_count: result.sample_count,
                    loss_count: result.loss_count,
                    measured_at_unix_ms: if result.measured_at_unix_ms == 0 {
                        now_unix_ms()
                    } else {
                        result.measured_at_unix_ms
                    },
                });
                publish_link(events, peer_node_id, link).await;
            } else if result.kind == BenchmarkKind::Bandwidth as i32 {
                let measurement = BandwidthMeasurement {
                    bandwidth_bps: result.bandwidth_bps,
                    payload_bytes: result.payload_bytes,
                    transfer_ms: result.transfer_ms,
                    measured_at_unix_ms: if result.measured_at_unix_ms == 0 {
                        now_unix_ms()
                    } else {
                        result.measured_at_unix_ms
                    },
                };
                if result.direction == BenchmarkDirection::ToPeer as i32 {
                    link.from_peer_bandwidth = Some(measurement);
                } else if result.direction == BenchmarkDirection::FromPeer as i32 {
                    link.to_peer_bandwidth = Some(measurement);
                }
                publish_link(events, peer_node_id, link).await;
            }
            Ok(())
        }
        Some(Body::PeerUpdate(update)) => {
            let mut peers = Vec::with_capacity(update.peers.len().min(64));
            for peer in update.peers.into_iter().take(64) {
                match peer_from_proto(peer) {
                    Ok(peer) => peers.push(peer),
                    Err(error) => warn!(%error, "ignored invalid peer update record"),
                }
            }
            if !peers.is_empty() {
                let _ = events
                    .send(SessionEvent::PeerUpdate {
                        from_peer: peer_node_id,
                        peers,
                    })
                    .await;
            }
            Ok(())
        }
        Some(Body::IntroductionOffer(offer)) => {
            handle_introduction_offer(peer_node_id, events, offer).await
        }
        Some(Body::IntroductionReady(ready)) => {
            handle_introduction_ready(peer_node_id, events, ready).await
        }
        Some(Body::PeerObserve(observe)) => {
            handle_peer_observe(peer_node_id, events, observe).await
        }
        Some(Body::Heartbeat(_)) | None => Ok(()),
        Some(other) => {
            warn!(?other, "ignored control message on established session");
            Ok(())
        }
    }
}

async fn handle_introduction_offer(
    from_peer: NodeId,
    events: &mpsc::Sender<SessionEvent>,
    offer: IntroductionOffer,
) -> NetResult<()> {
    let target_node_id = NodeId::from_slice(&offer.target_node_id)?;
    let attempt_id = attempt_id_from_bytes(&offer.attempt_id)?;
    let observed_address = parse_socket_addr(&offer.observed_address)?;
    let _ = events
        .send(SessionEvent::IntroductionOffer {
            from_peer,
            target_node_id,
            attempt_id,
            start_at_unix_ms: offer.start_at_unix_ms,
            observed_address,
        })
        .await;
    Ok(())
}

async fn handle_introduction_ready(
    from_peer: NodeId,
    events: &mpsc::Sender<SessionEvent>,
    ready: IntroductionReady,
) -> NetResult<()> {
    let peer_node_id = NodeId::from_slice(&ready.peer_node_id)?;
    let attempt_id = attempt_id_from_bytes(&ready.attempt_id)?;
    let peer_observed = parse_socket_addr(&ready.peer_observed_address)?;
    let self_observed = parse_socket_addr(&ready.self_observed_address)?;
    let _ = events
        .send(SessionEvent::IntroductionReady {
            from_peer,
            attempt_id,
            peer_node_id,
            peer_observed,
            self_observed,
            start_at_unix_ms: ready.start_at_unix_ms,
        })
        .await;
    Ok(())
}

async fn handle_peer_observe(
    from_peer: NodeId,
    events: &mpsc::Sender<SessionEvent>,
    observe: PeerObserve,
) -> NetResult<()> {
    let observed_node_id = NodeId::from_slice(&observe.observed_node_id)?;
    let address = parse_socket_addr(&observe.address)?;
    let _ = events
        .send(SessionEvent::PeerObserve {
            from_peer,
            observed_node_id,
            address,
            observed_at_unix_ms: if observe.observed_at_unix_ms == 0 {
                now_unix_ms()
            } else {
                observe.observed_at_unix_ms
            },
        })
        .await;
    Ok(())
}

async fn handle_benchmark_request(
    identity: &LocalIdentity,
    peer_node_id: NodeId,
    connection: &Connection,
    send: &mut SendStream,
    recv: &mut RecvStream,
    link: &mut LinkMeasurement,
    events: &mpsc::Sender<SessionEvent>,
    request: BenchmarkRequest,
) -> NetResult<()> {
    match BenchmarkKind::try_from(request.kind).unwrap_or(BenchmarkKind::Unspecified) {
        BenchmarkKind::Delay => {
            let measurement =
                respond_delay_benchmark(identity, send, recv, request.probe_id).await?;
            link.delay = Some(measurement);
            publish_link(events, peer_node_id, link).await;
            Ok(())
        }
        BenchmarkKind::Bandwidth => {
            match BenchmarkDirection::try_from(request.direction)
                .unwrap_or(BenchmarkDirection::Unspecified)
            {
                BenchmarkDirection::ToPeer => {
                    let outcome = respond_bandwidth_receive(
                        identity,
                        connection,
                        send,
                        request.probe_id,
                        request.payload_bytes,
                    )
                    .await?;
                    link.from_peer_bandwidth = Some(BandwidthMeasurement {
                        bandwidth_bps: outcome.bandwidth_bps,
                        payload_bytes: outcome.payload_bytes,
                        transfer_ms: outcome.transfer_ms,
                        measured_at_unix_ms: outcome.measured_at_unix_ms,
                    });
                    publish_link(events, peer_node_id, link).await;
                    Ok(())
                }
                BenchmarkDirection::FromPeer => {
                    respond_bandwidth_send(
                        identity,
                        connection,
                        send,
                        request.probe_id,
                        request.payload_bytes,
                    )
                    .await?;
                    Ok(())
                }
                BenchmarkDirection::Unspecified => Err(NetError::Protocol(
                    "bandwidth benchmark missing direction".to_owned(),
                )),
            }
        }
        BenchmarkKind::Unspecified => Err(NetError::Protocol(
            "benchmark request missing kind".to_owned(),
        )),
    }
}

async fn publish_link(
    events: &mpsc::Sender<SessionEvent>,
    peer_node_id: NodeId,
    link: &LinkMeasurement,
) {
    let _ = events
        .send(SessionEvent::Link {
            peer_node_id,
            measurement: link.clone(),
        })
        .await;
}

pub fn build_peer_update_envelope(
    identity: &LocalIdentity,
    peers: &[PeerRecord],
) -> ControlEnvelope {
    ControlEnvelope {
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor: PROTOCOL_MINOR,
        message_id: random_message_id(),
        sender_node_id: identity.node_id.to_vec().into(),
        in_reply_to: None,
        body: Some(Body::PeerUpdate(PeerUpdate {
            peers: peers.iter().map(peer_to_proto).collect(),
        })),
    }
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
    let candidates = candidates_from_proto(&peer.candidates)?;
    Ok(PeerRecord::new(
        node_id,
        peer.display_name,
        peer.certificate_der.to_vec(),
        candidates,
    ))
}

fn attempt_id_from_bytes(bytes: &[u8]) -> NetResult<[u8; 16]> {
    bytes
        .try_into()
        .map_err(|_| NetError::Protocol("introduction attempt id must be 16 bytes".to_owned()))
}

#[allow(dead_code)]
pub fn observed_candidate_from_remote(
    address: std::net::SocketAddr,
    source: NodeId,
) -> EndpointCandidate {
    peer_observed_candidate(address, source)
}
