use mesh_core::protocol::proto::{
    BenchmarkDirection, BenchmarkKind, BenchmarkRequest, ControlEnvelope, control_envelope::Body,
};
use mesh_core::{
    BandwidthMeasurement, CapabilityReport, DelayMeasurement, LinkMeasurement, LocalIdentity,
    NodeId, now_unix_ms,
};
use quinn::{Connection, RecvStream, SendStream};
use tokio::sync::mpsc;
use tracing::warn;

use crate::benchmark::{
    build_capability_envelope, capability_from_proto, default_bandwidth_payload,
    respond_bandwidth_receive, respond_bandwidth_send, respond_delay_benchmark,
    run_bandwidth_receive, run_bandwidth_send, run_delay_benchmark,
};
use crate::frame::{read_envelope, write_envelope};
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
    Failed {
        peer_node_id: NodeId,
        message: String,
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
        let envelope = match read_envelope(recv).await {
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
        Some(Body::Heartbeat(_)) | Some(Body::PeerUpdate(_)) | None => Ok(()),
        Some(other) => {
            warn!(?other, "ignored control message on established session");
            Ok(())
        }
    }
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
