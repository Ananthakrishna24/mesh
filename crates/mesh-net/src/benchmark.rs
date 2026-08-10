use std::time::{Duration, Instant};

use bytes::Bytes;
use mesh_core::protocol::proto::{
    BenchmarkAccept, BenchmarkDirection, BenchmarkKind, BenchmarkReject, BenchmarkRequest,
    BenchmarkResult, ControlEnvelope, Heartbeat, control_envelope::Body,
};
use mesh_core::{
    DEFAULT_BANDWIDTH_PAYLOAD_BYTES, DelayMeasurement, LocalIdentity, MAX_BANDWIDTH_PAYLOAD_BYTES,
    MIN_BANDWIDTH_PAYLOAD_BYTES, PROTOCOL_MAJOR, PROTOCOL_MINOR, now_unix_ms, random_message_id,
    stability_score,
};
use quinn::{Connection, RecvStream, SendStream};
use tokio::time::timeout;

use crate::frame::{read_envelope, write_envelope};
use crate::{NetError, NetResult};

pub const BENCHMARK_STREAM_MAGIC: &[u8; 4] = b"MSHB";
pub const BENCHMARK_HEADER_LEN: usize = 32;
pub const DELAY_PROBE_COUNT: usize = 7;
pub const DELAY_SPACING: Duration = Duration::from_millis(20);
pub const DELAY_PROBE_TIMEOUT: Duration = Duration::from_millis(1_000);
pub const DELAY_TOTAL_TIMEOUT: Duration = Duration::from_secs(5);
pub const BANDWIDTH_TOTAL_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct DelayBenchmarkOutcome {
    pub measurement: DelayMeasurement,
    pub probe_id: [u8; 16],
}

#[derive(Debug, Clone)]
pub struct BandwidthBenchmarkOutcome {
    pub bandwidth_bps: u64,
    pub payload_bytes: u64,
    pub transfer_ms: f64,
    pub measured_at_unix_ms: i64,
    pub probe_id: [u8; 16],
    pub sent_by_local: bool,
}

pub fn build_capability_envelope(
    identity: &LocalIdentity,
    report: &mesh_core::CapabilityReport,
) -> ControlEnvelope {
    ControlEnvelope {
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor: PROTOCOL_MINOR,
        message_id: random_message_id(),
        sender_node_id: identity.node_id.to_vec().into(),
        in_reply_to: None,
        body: Some(Body::CapabilityReport(capability_to_proto(report))),
    }
}

pub fn capability_to_proto(
    report: &mesh_core::CapabilityReport,
) -> mesh_core::protocol::proto::CapabilityReport {
    use mesh_core::protocol::proto::{GpuBackend, GpuDevice};

    mesh_core::protocol::proto::CapabilityReport {
        collected_at_unix_ms: report.collected_at_unix_ms,
        os: report.os.clone(),
        arch: report.arch.clone(),
        cpu_model: report.cpu_model.clone(),
        cpu_logical_cores: report.cpu_logical_cores,
        memory_total_bytes: report.memory_total_bytes,
        memory_available_bytes: report.memory_available_bytes,
        disk_total_bytes: report.disk_total_bytes,
        disk_available_bytes: report.disk_available_bytes,
        gpus: report
            .gpus
            .iter()
            .map(|gpu| GpuDevice {
                backend: match gpu.backend {
                    mesh_core::GpuBackendKind::Cuda => GpuBackend::Cuda as i32,
                    mesh_core::GpuBackendKind::Metal => GpuBackend::Metal as i32,
                },
                stable_id: gpu.stable_id.clone(),
                name: gpu.name.clone(),
                total_memory_bytes: gpu.total_memory_bytes,
                available_memory_bytes: gpu.available_memory_bytes,
                driver_version: gpu.driver_version.clone(),
                runtime_version: gpu.runtime_version.clone(),
            })
            .collect(),
        cpu_fp32_gflops: report.compute.cpu_fp32_gflops,
        compute_measured_at_unix_ms: report.compute.measured_at_unix_ms,
        status: report.status.clone(),
    }
}

pub fn capability_from_proto(
    report: mesh_core::protocol::proto::CapabilityReport,
) -> NetResult<mesh_core::CapabilityReport> {
    use mesh_core::protocol::proto::GpuBackend;
    use mesh_core::{ComputeProxy, GpuBackendKind, GpuDeviceInfo};

    let mut gpus = Vec::with_capacity(report.gpus.len());
    for gpu in report.gpus {
        let backend = match GpuBackend::try_from(gpu.backend).unwrap_or(GpuBackend::Unspecified) {
            GpuBackend::Cuda => GpuBackendKind::Cuda,
            GpuBackend::Metal => GpuBackendKind::Metal,
            GpuBackend::Unspecified => {
                return Err(NetError::Protocol("unknown GPU backend".to_owned()));
            }
        };
        gpus.push(GpuDeviceInfo {
            backend,
            stable_id: gpu.stable_id,
            name: gpu.name,
            total_memory_bytes: gpu.total_memory_bytes,
            available_memory_bytes: gpu.available_memory_bytes,
            driver_version: gpu.driver_version,
            runtime_version: gpu.runtime_version,
        });
    }

    Ok(mesh_core::CapabilityReport {
        collected_at_unix_ms: report.collected_at_unix_ms,
        os: report.os,
        arch: report.arch,
        cpu_model: report.cpu_model,
        cpu_logical_cores: report.cpu_logical_cores,
        memory_total_bytes: report.memory_total_bytes,
        memory_available_bytes: report.memory_available_bytes,
        disk_total_bytes: report.disk_total_bytes,
        disk_available_bytes: report.disk_available_bytes,
        gpus,
        compute: ComputeProxy {
            cpu_fp32_gflops: report.cpu_fp32_gflops,
            measured_at_unix_ms: report.compute_measured_at_unix_ms,
        },
        status: report.status,
    })
}

pub async fn run_delay_benchmark(
    identity: &LocalIdentity,
    send: &mut SendStream,
    recv: &mut RecvStream,
) -> NetResult<DelayBenchmarkOutcome> {
    let probe_id = random_probe_id();
    let request = ControlEnvelope {
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor: PROTOCOL_MINOR,
        message_id: random_message_id(),
        sender_node_id: identity.node_id.to_vec().into(),
        in_reply_to: None,
        body: Some(Body::BenchmarkRequest(BenchmarkRequest {
            probe_id: Bytes::copy_from_slice(&probe_id),
            kind: BenchmarkKind::Delay as i32,
            direction: BenchmarkDirection::Unspecified as i32,
            payload_bytes: 0,
        })),
    };
    write_envelope(send, &request).await?;

    let accept = timeout(DELAY_TOTAL_TIMEOUT, read_envelope(recv))
        .await
        .map_err(|_| NetError::Timeout)??;
    match accept.body {
        Some(Body::BenchmarkAccept(BenchmarkAccept { probe_id: accepted }))
            if accepted.as_ref() == probe_id => {}
        Some(Body::BenchmarkReject(BenchmarkReject { reason, .. })) => {
            return Err(NetError::Protocol(format!(
                "delay benchmark rejected: {reason}"
            )));
        }
        _ => {
            return Err(NetError::Protocol(
                "expected delay benchmark accept".to_owned(),
            ));
        }
    }

    let mut samples = Vec::with_capacity(DELAY_PROBE_COUNT);
    let mut loss_count = 0u32;
    for _ in 0..DELAY_PROBE_COUNT {
        let message_id = random_message_id();
        let heartbeat = ControlEnvelope {
            protocol_major: PROTOCOL_MAJOR,
            protocol_minor: PROTOCOL_MINOR,
            message_id: message_id.clone(),
            sender_node_id: identity.node_id.to_vec().into(),
            in_reply_to: None,
            body: Some(Body::Heartbeat(Heartbeat {
                sent_at_unix_ms: now_unix_ms(),
            })),
        };
        let sent_at = Instant::now();
        write_envelope(send, &heartbeat).await?;
        match timeout(DELAY_PROBE_TIMEOUT, read_envelope(recv)).await {
            Ok(Ok(reply)) => {
                let matches = reply
                    .in_reply_to
                    .as_ref()
                    .is_some_and(|value| value.as_ref() == message_id.as_ref());
                if matches && matches!(reply.body, Some(Body::Heartbeat(_))) {
                    samples.push(sent_at.elapsed().as_secs_f64() * 1_000.0);
                } else {
                    loss_count += 1;
                }
            }
            _ => loss_count += 1,
        }
        tokio::time::sleep(DELAY_SPACING).await;
    }

    let measurement = summarize_delay_samples(&samples, loss_count)?;
    let result = ControlEnvelope {
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor: PROTOCOL_MINOR,
        message_id: random_message_id(),
        sender_node_id: identity.node_id.to_vec().into(),
        in_reply_to: None,
        body: Some(Body::BenchmarkResult(BenchmarkResult {
            probe_id: Bytes::copy_from_slice(&probe_id),
            kind: BenchmarkKind::Delay as i32,
            direction: BenchmarkDirection::Unspecified as i32,
            one_way_delay_ms: measurement.one_way_delay_ms,
            rtt_ms: measurement.rtt_ms,
            rtt_p95_ms: measurement.rtt_p95_ms,
            stability_score: u32::from(measurement.stability_score),
            sample_count: measurement.sample_count,
            loss_count: measurement.loss_count,
            bandwidth_bps: 0,
            payload_bytes: 0,
            transfer_ms: 0.0,
            measured_at_unix_ms: measurement.measured_at_unix_ms,
        })),
    };
    write_envelope(send, &result).await?;

    Ok(DelayBenchmarkOutcome {
        measurement,
        probe_id,
    })
}

pub async fn respond_delay_benchmark(
    identity: &LocalIdentity,
    send: &mut SendStream,
    recv: &mut RecvStream,
    probe_id: Bytes,
) -> NetResult<DelayMeasurement> {
    let accept = ControlEnvelope {
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor: PROTOCOL_MINOR,
        message_id: random_message_id(),
        sender_node_id: identity.node_id.to_vec().into(),
        in_reply_to: None,
        body: Some(Body::BenchmarkAccept(BenchmarkAccept {
            probe_id: probe_id.clone(),
        })),
    };
    write_envelope(send, &accept).await?;

    let deadline = Instant::now() + DELAY_TOTAL_TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(NetError::Timeout);
        }
        let envelope = timeout(remaining, read_envelope(recv))
            .await
            .map_err(|_| NetError::Timeout)??;
        match envelope.body {
            Some(Body::Heartbeat(_)) => {
                let reply = ControlEnvelope {
                    protocol_major: PROTOCOL_MAJOR,
                    protocol_minor: PROTOCOL_MINOR,
                    message_id: random_message_id(),
                    sender_node_id: identity.node_id.to_vec().into(),
                    in_reply_to: Some(envelope.message_id),
                    body: Some(Body::Heartbeat(Heartbeat {
                        sent_at_unix_ms: now_unix_ms(),
                    })),
                };
                write_envelope(send, &reply).await?;
            }
            Some(Body::BenchmarkResult(result))
                if result.probe_id.as_ref() == probe_id.as_ref()
                    && result.kind == BenchmarkKind::Delay as i32 =>
            {
                return Ok(DelayMeasurement {
                    one_way_delay_ms: result.one_way_delay_ms,
                    rtt_ms: result.rtt_ms,
                    rtt_p95_ms: result.rtt_p95_ms,
                    stability_score: result.stability_score.min(u32::from(u8::MAX)) as u8,
                    sample_count: result.sample_count,
                    loss_count: result.loss_count,
                    measured_at_unix_ms: result.measured_at_unix_ms,
                });
            }
            Some(Body::BenchmarkReject(reject)) => {
                return Err(NetError::Protocol(format!(
                    "delay benchmark failed: {}",
                    reject.reason
                )));
            }
            _ => {
                return Err(NetError::Protocol(
                    "unexpected message during delay response".to_owned(),
                ));
            }
        }
    }
}

pub async fn run_bandwidth_send(
    identity: &LocalIdentity,
    connection: &Connection,
    send: &mut SendStream,
    recv: &mut RecvStream,
    payload_bytes: u64,
) -> NetResult<BandwidthBenchmarkOutcome> {
    let payload_bytes =
        payload_bytes.clamp(MIN_BANDWIDTH_PAYLOAD_BYTES, MAX_BANDWIDTH_PAYLOAD_BYTES);
    let probe_id = random_probe_id();
    let request = ControlEnvelope {
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor: PROTOCOL_MINOR,
        message_id: random_message_id(),
        sender_node_id: identity.node_id.to_vec().into(),
        in_reply_to: None,
        body: Some(Body::BenchmarkRequest(BenchmarkRequest {
            probe_id: Bytes::copy_from_slice(&probe_id),
            kind: BenchmarkKind::Bandwidth as i32,
            direction: BenchmarkDirection::ToPeer as i32,
            payload_bytes,
        })),
    };
    write_envelope(send, &request).await?;
    let accept = timeout(BANDWIDTH_TOTAL_TIMEOUT, read_envelope(recv))
        .await
        .map_err(|_| NetError::Timeout)??;
    match accept.body {
        Some(Body::BenchmarkAccept(BenchmarkAccept { probe_id: accepted }))
            if accepted.as_ref() == probe_id => {}
        Some(Body::BenchmarkReject(BenchmarkReject { reason, .. })) => {
            return Err(NetError::Protocol(format!(
                "bandwidth benchmark rejected: {reason}"
            )));
        }
        _ => {
            return Err(NetError::Protocol(
                "expected bandwidth benchmark accept".to_owned(),
            ));
        }
    }

    let mut stream = connection.open_uni().await?;
    let header = encode_benchmark_header(&probe_id, payload_bytes);
    let chunk = vec![0u8; 64 * 1024];
    let mut remaining = payload_bytes;
    let started = Instant::now();
    stream.write_all(&header).await?;
    while remaining > 0 {
        let n = remaining.min(chunk.len() as u64) as usize;
        stream.write_all(&chunk[..n]).await?;
        remaining -= n as u64;
    }
    stream.finish()?;
    let _ = stream.stopped().await;
    let transfer_ms = started.elapsed().as_secs_f64() * 1_000.0;

    let result = timeout(BANDWIDTH_TOTAL_TIMEOUT, read_envelope(recv))
        .await
        .map_err(|_| NetError::Timeout)??;
    match result.body {
        Some(Body::BenchmarkResult(result))
            if result.probe_id.as_ref() == probe_id
                && result.kind == BenchmarkKind::Bandwidth as i32 =>
        {
            Ok(BandwidthBenchmarkOutcome {
                bandwidth_bps: result.bandwidth_bps,
                payload_bytes: result.payload_bytes,
                transfer_ms: result.transfer_ms.max(transfer_ms),
                measured_at_unix_ms: result.measured_at_unix_ms,
                probe_id,
                sent_by_local: true,
            })
        }
        _ => Err(NetError::Protocol(
            "expected bandwidth benchmark result".to_owned(),
        )),
    }
}

pub async fn run_bandwidth_receive(
    identity: &LocalIdentity,
    connection: &Connection,
    send: &mut SendStream,
    recv: &mut RecvStream,
    payload_bytes: u64,
) -> NetResult<BandwidthBenchmarkOutcome> {
    let payload_bytes =
        payload_bytes.clamp(MIN_BANDWIDTH_PAYLOAD_BYTES, MAX_BANDWIDTH_PAYLOAD_BYTES);
    let probe_id = random_probe_id();
    let request = ControlEnvelope {
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor: PROTOCOL_MINOR,
        message_id: random_message_id(),
        sender_node_id: identity.node_id.to_vec().into(),
        in_reply_to: None,
        body: Some(Body::BenchmarkRequest(BenchmarkRequest {
            probe_id: Bytes::copy_from_slice(&probe_id),
            kind: BenchmarkKind::Bandwidth as i32,
            direction: BenchmarkDirection::FromPeer as i32,
            payload_bytes,
        })),
    };
    write_envelope(send, &request).await?;
    let accept = timeout(BANDWIDTH_TOTAL_TIMEOUT, read_envelope(recv))
        .await
        .map_err(|_| NetError::Timeout)??;
    match accept.body {
        Some(Body::BenchmarkAccept(BenchmarkAccept { probe_id: accepted }))
            if accepted.as_ref() == probe_id => {}
        Some(Body::BenchmarkReject(BenchmarkReject { reason, .. })) => {
            return Err(NetError::Protocol(format!(
                "bandwidth receive rejected: {reason}"
            )));
        }
        _ => {
            return Err(NetError::Protocol(
                "expected bandwidth receive accept".to_owned(),
            ));
        }
    }

    let outcome = receive_benchmark_stream(connection, &probe_id, payload_bytes).await?;
    let result = ControlEnvelope {
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor: PROTOCOL_MINOR,
        message_id: random_message_id(),
        sender_node_id: identity.node_id.to_vec().into(),
        in_reply_to: None,
        body: Some(Body::BenchmarkResult(BenchmarkResult {
            probe_id: Bytes::copy_from_slice(&probe_id),
            kind: BenchmarkKind::Bandwidth as i32,
            direction: BenchmarkDirection::FromPeer as i32,
            one_way_delay_ms: 0.0,
            rtt_ms: 0.0,
            rtt_p95_ms: 0.0,
            stability_score: 0,
            sample_count: 0,
            loss_count: 0,
            bandwidth_bps: outcome.bandwidth_bps,
            payload_bytes: outcome.payload_bytes,
            transfer_ms: outcome.transfer_ms,
            measured_at_unix_ms: outcome.measured_at_unix_ms,
        })),
    };
    write_envelope(send, &result).await?;
    Ok(BandwidthBenchmarkOutcome {
        probe_id,
        sent_by_local: false,
        ..outcome
    })
}

pub async fn respond_bandwidth_receive(
    identity: &LocalIdentity,
    connection: &Connection,
    send: &mut SendStream,
    probe_id: Bytes,
    payload_bytes: u64,
) -> NetResult<BandwidthBenchmarkOutcome> {
    let payload_bytes =
        payload_bytes.clamp(MIN_BANDWIDTH_PAYLOAD_BYTES, MAX_BANDWIDTH_PAYLOAD_BYTES);
    let accept = ControlEnvelope {
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor: PROTOCOL_MINOR,
        message_id: random_message_id(),
        sender_node_id: identity.node_id.to_vec().into(),
        in_reply_to: None,
        body: Some(Body::BenchmarkAccept(BenchmarkAccept {
            probe_id: probe_id.clone(),
        })),
    };
    write_envelope(send, &accept).await?;

    let mut probe = [0u8; 16];
    if probe_id.len() != 16 {
        return Err(NetError::Protocol("invalid probe id".to_owned()));
    }
    probe.copy_from_slice(probe_id.as_ref());
    let outcome = receive_benchmark_stream(connection, &probe, payload_bytes).await?;
    let result = ControlEnvelope {
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor: PROTOCOL_MINOR,
        message_id: random_message_id(),
        sender_node_id: identity.node_id.to_vec().into(),
        in_reply_to: None,
        body: Some(Body::BenchmarkResult(BenchmarkResult {
            probe_id,
            kind: BenchmarkKind::Bandwidth as i32,
            direction: BenchmarkDirection::FromPeer as i32,
            one_way_delay_ms: 0.0,
            rtt_ms: 0.0,
            rtt_p95_ms: 0.0,
            stability_score: 0,
            sample_count: 0,
            loss_count: 0,
            bandwidth_bps: outcome.bandwidth_bps,
            payload_bytes: outcome.payload_bytes,
            transfer_ms: outcome.transfer_ms,
            measured_at_unix_ms: outcome.measured_at_unix_ms,
        })),
    };
    write_envelope(send, &result).await?;
    Ok(BandwidthBenchmarkOutcome {
        probe_id: probe,
        sent_by_local: false,
        ..outcome
    })
}

pub async fn respond_bandwidth_send(
    identity: &LocalIdentity,
    connection: &Connection,
    send: &mut SendStream,
    probe_id: Bytes,
    payload_bytes: u64,
) -> NetResult<()> {
    let payload_bytes =
        payload_bytes.clamp(MIN_BANDWIDTH_PAYLOAD_BYTES, MAX_BANDWIDTH_PAYLOAD_BYTES);
    let accept = ControlEnvelope {
        protocol_major: PROTOCOL_MAJOR,
        protocol_minor: PROTOCOL_MINOR,
        message_id: random_message_id(),
        sender_node_id: identity.node_id.to_vec().into(),
        in_reply_to: None,
        body: Some(Body::BenchmarkAccept(BenchmarkAccept {
            probe_id: probe_id.clone(),
        })),
    };
    write_envelope(send, &accept).await?;

    let mut probe = [0u8; 16];
    if probe_id.len() != 16 {
        return Err(NetError::Protocol("invalid probe id".to_owned()));
    }
    probe.copy_from_slice(probe_id.as_ref());
    let mut stream = connection.open_uni().await?;
    let header = encode_benchmark_header(&probe, payload_bytes);
    let chunk = vec![0u8; 64 * 1024];
    let mut remaining = payload_bytes;
    stream.write_all(&header).await?;
    while remaining > 0 {
        let n = remaining.min(chunk.len() as u64) as usize;
        stream.write_all(&chunk[..n]).await?;
        remaining -= n as u64;
    }
    stream.finish()?;
    let _ = stream.stopped().await;
    Ok(())
}

pub fn default_bandwidth_payload() -> u64 {
    DEFAULT_BANDWIDTH_PAYLOAD_BYTES
}

pub fn summarize_delay_samples(samples: &[f64], loss_count: u32) -> NetResult<DelayMeasurement> {
    if samples.len() < 3 {
        return Err(NetError::Protocol(
            "delay benchmark produced too few samples".to_owned(),
        ));
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    let retained = if sorted.len() >= 5 {
        sorted[1..sorted.len() - 1].to_vec()
    } else {
        sorted
    };
    let rtt_ms = retained.iter().sum::<f64>() / retained.len() as f64;
    let index = ((retained.len() as f64) * 0.95).ceil() as usize;
    let p95_index = index.saturating_sub(1).min(retained.len() - 1);
    let rtt_p95_ms = retained[p95_index];
    Ok(DelayMeasurement {
        one_way_delay_ms: rtt_ms / 2.0,
        rtt_ms,
        rtt_p95_ms,
        stability_score: stability_score(&retained, loss_count),
        sample_count: samples.len() as u32,
        loss_count,
        measured_at_unix_ms: now_unix_ms(),
    })
}

fn encode_benchmark_header(probe_id: &[u8; 16], payload_bytes: u64) -> [u8; BENCHMARK_HEADER_LEN] {
    let mut header = [0u8; BENCHMARK_HEADER_LEN];
    header[0..4].copy_from_slice(BENCHMARK_STREAM_MAGIC);
    header[4..6].copy_from_slice(&1u16.to_be_bytes());
    header[8..24].copy_from_slice(probe_id);
    header[24..32].copy_from_slice(&payload_bytes.to_be_bytes());
    header
}

async fn receive_benchmark_stream(
    connection: &Connection,
    expected_probe: &[u8; 16],
    expected_payload: u64,
) -> NetResult<BandwidthBenchmarkOutcome> {
    let mut stream = timeout(BANDWIDTH_TOTAL_TIMEOUT, connection.accept_uni())
        .await
        .map_err(|_| NetError::Timeout)??;
    let mut header = [0u8; BENCHMARK_HEADER_LEN];
    stream.read_exact(&mut header).await?;
    if &header[0..4] != BENCHMARK_STREAM_MAGIC {
        return Err(NetError::Protocol(
            "invalid benchmark stream magic".to_owned(),
        ));
    }
    let version = u16::from_be_bytes([header[4], header[5]]);
    if version != 1 {
        return Err(NetError::Protocol(format!(
            "unsupported benchmark header version {version}"
        )));
    }
    let mut probe = [0u8; 16];
    probe.copy_from_slice(&header[8..24]);
    if &probe != expected_probe {
        return Err(NetError::Protocol("benchmark probe id mismatch".to_owned()));
    }
    let payload_bytes = u64::from_be_bytes(header[24..32].try_into().unwrap_or([0; 8]));
    if payload_bytes != expected_payload {
        return Err(NetError::Protocol(
            "benchmark payload length mismatch".to_owned(),
        ));
    }
    if payload_bytes < MIN_BANDWIDTH_PAYLOAD_BYTES || payload_bytes > MAX_BANDWIDTH_PAYLOAD_BYTES {
        return Err(NetError::Protocol(
            "benchmark payload outside accepted limits".to_owned(),
        ));
    }

    let mut remaining = payload_bytes;
    let mut buffer = vec![0u8; 64 * 1024];
    let started = Instant::now();
    while remaining > 0 {
        let n = remaining.min(buffer.len() as u64) as usize;
        stream.read_exact(&mut buffer[..n]).await?;
        remaining -= n as u64;
    }
    let transfer_ms = started.elapsed().as_secs_f64() * 1_000.0;
    let transfer_secs = (transfer_ms / 1_000.0).max(1e-6);
    let bandwidth_bps = ((payload_bytes as f64) * 8.0 / transfer_secs) as u64;
    Ok(BandwidthBenchmarkOutcome {
        bandwidth_bps,
        payload_bytes,
        transfer_ms,
        measured_at_unix_ms: now_unix_ms(),
        probe_id: probe,
        sent_by_local: false,
    })
}

fn random_probe_id() -> [u8; 16] {
    let bytes = random_message_id();
    let mut probe = [0u8; 16];
    probe.copy_from_slice(bytes.as_ref());
    probe
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_summary_discards_extremes() {
        let samples = [10.0, 11.0, 12.0, 13.0, 50.0, 9.0, 10.5];
        let measurement = summarize_delay_samples(&samples, 0).expect("summary");
        assert!(measurement.rtt_ms < 20.0);
        assert!(measurement.stability_score > 50);
        assert_eq!(measurement.sample_count, 7);
    }
}
