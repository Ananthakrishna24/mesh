use serde::{Deserialize, Serialize};

use crate::NodeId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GpuBackendKind {
    Cuda,
    Metal,
}

impl GpuBackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cuda => "cuda",
            Self::Metal => "metal",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GpuDeviceInfo {
    pub backend: GpuBackendKind,
    pub stable_id: String,
    pub name: String,
    pub total_memory_bytes: u64,
    pub available_memory_bytes: Option<u64>,
    pub driver_version: Option<String>,
    pub runtime_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComputeProxy {
    pub cpu_fp32_gflops: f64,
    pub measured_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityReport {
    pub collected_at_unix_ms: i64,
    pub os: String,
    pub arch: String,
    pub cpu_model: String,
    pub cpu_logical_cores: u32,
    pub memory_total_bytes: u64,
    pub memory_available_bytes: u64,
    pub disk_total_bytes: u64,
    pub disk_available_bytes: u64,
    pub gpus: Vec<GpuDeviceInfo>,
    pub compute: ComputeProxy,
    pub status: String,
}

impl CapabilityReport {
    pub fn summary_line(&self) -> String {
        let gpu = if self.gpus.is_empty() {
            "no GPU".to_owned()
        } else {
            self.gpus
                .iter()
                .map(|gpu| {
                    format!(
                        "{} {} ({})",
                        gpu.backend.as_str(),
                        gpu.name,
                        format_bytes(gpu.total_memory_bytes)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        format!(
            "{} · {} cores · RAM {} free / {} · disk {} free · {}",
            self.cpu_model,
            self.cpu_logical_cores,
            format_bytes(self.memory_available_bytes),
            format_bytes(self.memory_total_bytes),
            format_bytes(self.disk_available_bytes),
            gpu
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MeasurementAgeState {
    Fresh,
    Stale,
    Expired,
    Missing,
}

impl MeasurementAgeState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Stale => "stale",
            Self::Expired => "expired",
            Self::Missing => "unavailable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DelayMeasurement {
    pub one_way_delay_ms: f64,
    pub rtt_ms: f64,
    pub rtt_p95_ms: f64,
    pub stability_score: u8,
    pub sample_count: u32,
    pub loss_count: u32,
    pub measured_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BandwidthMeasurement {
    pub bandwidth_bps: u64,
    pub payload_bytes: u64,
    pub transfer_ms: f64,
    pub measured_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkMeasurement {
    pub peer_node_id: NodeId,
    pub delay: Option<DelayMeasurement>,
    pub to_peer_bandwidth: Option<BandwidthMeasurement>,
    pub from_peer_bandwidth: Option<BandwidthMeasurement>,
}

impl LinkMeasurement {
    pub fn empty(peer_node_id: NodeId) -> Self {
        Self {
            peer_node_id,
            delay: None,
            to_peer_bandwidth: None,
            from_peer_bandwidth: None,
        }
    }
}

pub const MEASUREMENT_FRESH_MS: i64 = 5 * 60 * 1000;
pub const MEASUREMENT_STALE_MS: i64 = 30 * 60 * 1000;
pub const DELAY_REJECT_ONE_WAY_MS: f64 = 80.0;
pub const BANDWIDTH_REJECT_BPS: u64 = 10_000_000;
pub const STABILITY_PIPELINE_MIN: u8 = 50;
pub const MAX_WAN_PIPELINE_STAGES: u32 = 3;
pub const DEFAULT_BANDWIDTH_PAYLOAD_BYTES: u64 = 4 * 1024 * 1024;
pub const MAX_BANDWIDTH_PAYLOAD_BYTES: u64 = 16 * 1024 * 1024;
pub const MIN_BANDWIDTH_PAYLOAD_BYTES: u64 = 256 * 1024;

pub fn measurement_age_state(measured_at_unix_ms: Option<i64>, now_unix_ms: i64) -> MeasurementAgeState {
    let Some(measured_at_unix_ms) = measured_at_unix_ms else {
        return MeasurementAgeState::Missing;
    };
    let age = now_unix_ms.saturating_sub(measured_at_unix_ms);
    if age < MEASUREMENT_FRESH_MS {
        MeasurementAgeState::Fresh
    } else if age <= MEASUREMENT_STALE_MS {
        MeasurementAgeState::Stale
    } else {
        MeasurementAgeState::Expired
    }
}

pub fn age_delay_ms(one_way_delay_ms: f64, state: MeasurementAgeState) -> Option<f64> {
    match state {
        MeasurementAgeState::Fresh => Some(one_way_delay_ms),
        MeasurementAgeState::Stale => Some(one_way_delay_ms * 1.25),
        MeasurementAgeState::Expired | MeasurementAgeState::Missing => None,
    }
}

pub fn age_bandwidth_bps(bandwidth_bps: u64, state: MeasurementAgeState) -> Option<u64> {
    match state {
        MeasurementAgeState::Fresh => Some(bandwidth_bps),
        MeasurementAgeState::Stale => Some(((bandwidth_bps as f64) * 0.8) as u64),
        MeasurementAgeState::Expired | MeasurementAgeState::Missing => None,
    }
}

pub fn stability_score(rtt_samples_ms: &[f64], loss_count: u32) -> u8 {
    if rtt_samples_ms.is_empty() {
        return 0;
    }
    let mean = rtt_samples_ms.iter().sum::<f64>() / rtt_samples_ms.len() as f64;
    let variance = rtt_samples_ms
        .iter()
        .map(|sample| {
            let delta = sample - mean;
            delta * delta
        })
        .sum::<f64>()
        / rtt_samples_ms.len() as f64;
    let stddev = variance.sqrt();
    let variance_penalty = (200.0 * stddev / mean.max(1.0)).round().min(40.0);
    let score = 100.0 - 4.0 * f64::from(loss_count) - variance_penalty;
    score.clamp(0.0, 100.0).round() as u8
}

pub fn pipeline_hop_rejects(
    one_way_delay_ms: Option<f64>,
    bandwidth_bps: Option<u64>,
    stability: Option<u8>,
) -> bool {
    match (one_way_delay_ms, bandwidth_bps, stability) {
        (Some(delay), Some(bandwidth), Some(score)) => {
            delay > DELAY_REJECT_ONE_WAY_MS
                || bandwidth < BANDWIDTH_REJECT_BPS
                || score < STABILITY_PIPELINE_MIN
        }
        _ => true,
    }
}

pub fn format_bytes(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    const TIB: f64 = GIB * 1024.0;
    let value = bytes as f64;
    if value >= TIB {
        format!("{:.1} TiB", value / TIB)
    } else if value >= GIB {
        format!("{:.1} GiB", value / GIB)
    } else if value >= MIB {
        format!("{:.1} MiB", value / MIB)
    } else if value >= KIB {
        format!("{:.1} KiB", value / KIB)
    } else {
        format!("{bytes} B")
    }
}

pub fn format_bits_per_second(bps: u64) -> String {
    const KBPS: f64 = 1_000.0;
    const MBPS: f64 = KBPS * 1_000.0;
    const GBPS: f64 = MBPS * 1_000.0;
    let value = bps as f64;
    if value >= GBPS {
        format!("{:.2} Gbps", value / GBPS)
    } else if value >= MBPS {
        format!("{:.1} Mbps", value / MBPS)
    } else if value >= KBPS {
        format!("{:.0} kbps", value / KBPS)
    } else {
        format!("{bps} bps")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stability_penalizes_loss_and_jitter() {
        let stable = stability_score(&[10.0, 11.0, 10.5, 10.2, 10.8], 0);
        let lossy = stability_score(&[10.0, 11.0, 10.5, 10.2, 10.8], 3);
        let jittery = stability_score(&[10.0, 40.0, 12.0, 35.0, 11.0], 0);
        assert!(stable >= 80);
        assert!(lossy < stable);
        assert!(jittery < stable);
    }

    #[test]
    fn age_windows_match_contract() {
        let now = 1_000_000_i64;
        assert_eq!(
            measurement_age_state(Some(now - 60_000), now),
            MeasurementAgeState::Fresh
        );
        assert_eq!(
            measurement_age_state(Some(now - 10 * 60_000), now),
            MeasurementAgeState::Stale
        );
        assert_eq!(
            measurement_age_state(Some(now - 40 * 60_000), now),
            MeasurementAgeState::Expired
        );
        assert_eq!(measurement_age_state(None, now), MeasurementAgeState::Missing);
    }

    #[test]
    fn pipeline_hop_thresholds() {
        assert!(!pipeline_hop_rejects(Some(20.0), Some(50_000_000), Some(90)));
        assert!(pipeline_hop_rejects(Some(90.0), Some(50_000_000), Some(90)));
        assert!(pipeline_hop_rejects(Some(20.0), Some(5_000_000), Some(90)));
        assert!(pipeline_hop_rejects(Some(20.0), Some(50_000_000), Some(40)));
        assert!(pipeline_hop_rejects(None, Some(50_000_000), Some(90)));
    }
}
