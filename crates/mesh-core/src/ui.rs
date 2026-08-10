use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

use crate::{
    CapabilityReport, LinkMeasurement, MeasurementAgeState, MeshId, ModelStoreView, NodeId,
    PeerSummary, ResourceManagerView, format_bits_per_second, format_bytes, measurement_age_state,
};


#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppScreen {
    FirstRun,
    Enroll,
    Dashboard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimePhase {
    Starting,
    AwaitingOnboarding,
    Preparing,
    Connecting,
    Ready,
    Failed,
    ShuttingDown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecoveryAction {
    RetryAutomatic,
    ShowManualSteps,
    RegenerateInvitation,
    OpenFirewallHelp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManualForwardingGuide {
    pub local_udp_port: u16,
    pub local_address: Option<SocketAddr>,
    pub protocol: String,
    pub public_address_input: String,
    pub instructions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectivityRecovery {
    pub title: String,
    pub message: String,
    pub primary: RecoveryAction,
    pub secondary: Option<RecoveryAction>,
    pub technical_details: Vec<String>,
    pub manual: Option<ManualForwardingGuide>,
    pub show_manual: bool,
    pub show_firewall_help: bool,
    pub firewall_message: String,
}


#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalNodeSummary {
    pub display_name: String,
    pub node_id: Option<NodeId>,
    pub mesh_id: Option<MeshId>,
    pub listen_addr: Option<SocketAddr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrollmentProgress {
    pub steps: Vec<String>,
    pub current: String,
    pub invitation_text: Option<String>,
    pub error: Option<String>,
    pub recovery: Option<ConnectivityRecovery>,
    pub router_mapping_ok: Option<bool>,
}


#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum UiCommand {
    CreateMesh { display_name: String },
    OpenEnrollment,
    CancelEnrollment,
    SubmitInvitation { text: String },
    CreateInvitation,
    ClearInvitation,
    RefreshHardware,
    RetryAutomaticConnectivity,
    ShowManualForwarding,
    HideManualForwarding,
    SetManualPublicAddress { address: String },
    ApplyManualPublicAddress,
    ShowFirewallHelp,
    HideFirewallHelp,
    RunLocalReservationProbe,
    ReleaseAllLocalReservations,
    SelectModel { reference: crate::ModelReference },
    RefreshProviderAccess,
    SaveHuggingFaceToken { token: String },
    DeleteHuggingFaceToken,
    ProbeSelectedModel,
    PrepareSelectedModel,
    CancelModelWork,
    ClearModelCache,
    LoadSelectedModel,
    UnloadModel,
    Generate { prompt: String, max_new_tokens: u32, temperature: f32, seed: u64 },
    CancelGeneration,
    Shutdown,
}


#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HardwareSummaryView {
    pub line: String,
    pub status: String,
    pub cpu_model: String,
    pub cpu_logical_cores: u32,
    pub memory_total_bytes: u64,
    pub memory_available_bytes: u64,
    pub disk_total_bytes: u64,
    pub disk_available_bytes: u64,
    pub gpu_lines: Vec<String>,
    pub cpu_fp32_gflops: f64,
}

impl HardwareSummaryView {
    pub fn from_report(report: &CapabilityReport) -> Self {
        let gpu_lines = if report.gpus.is_empty() {
            vec!["No supported GPU discovered".to_owned()]
        } else {
            report
                .gpus
                .iter()
                .map(|gpu| {
                    let free = gpu
                        .available_memory_bytes
                        .map(format_bytes)
                        .unwrap_or_else(|| "unknown".to_owned());
                    format!(
                        "{} · {} · {} total · {} free",
                        gpu.backend.as_str(),
                        gpu.name,
                        format_bytes(gpu.total_memory_bytes),
                        free
                    )
                })
                .collect()
        };
        Self {
            line: report.summary_line(),
            status: report.status.clone(),
            cpu_model: report.cpu_model.clone(),
            cpu_logical_cores: report.cpu_logical_cores,
            memory_total_bytes: report.memory_total_bytes,
            memory_available_bytes: report.memory_available_bytes,
            disk_total_bytes: report.disk_total_bytes,
            disk_available_bytes: report.disk_available_bytes,
            gpu_lines,
            cpu_fp32_gflops: report.compute.cpu_fp32_gflops,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LinkSummaryView {
    pub delay_ms: Option<f64>,
    pub delay_age: MeasurementAgeState,
    pub to_peer_bandwidth: Option<u64>,
    pub to_peer_age: MeasurementAgeState,
    pub from_peer_bandwidth: Option<u64>,
    pub from_peer_age: MeasurementAgeState,
    pub stability_score: Option<u8>,
}

impl LinkSummaryView {
    pub fn from_measurement(link: Option<&LinkMeasurement>, now: i64) -> Self {
        let delay = link.and_then(|item| item.delay.as_ref());
        let to_peer = link.and_then(|item| item.to_peer_bandwidth.as_ref());
        let from_peer = link.and_then(|item| item.from_peer_bandwidth.as_ref());
        Self {
            delay_ms: delay.map(|item| item.one_way_delay_ms),
            delay_age: measurement_age_state(delay.map(|item| item.measured_at_unix_ms), now),
            to_peer_bandwidth: to_peer.map(|item| item.bandwidth_bps),
            to_peer_age: measurement_age_state(to_peer.map(|item| item.measured_at_unix_ms), now),
            from_peer_bandwidth: from_peer.map(|item| item.bandwidth_bps),
            from_peer_age: measurement_age_state(
                from_peer.map(|item| item.measured_at_unix_ms),
                now,
            ),
            stability_score: delay.map(|item| item.stability_score),
        }
    }

    pub fn delay_label(&self) -> String {
        match (self.delay_ms, self.delay_age) {
            (Some(ms), age) => format!("{ms:.1} ms ({})", age.as_str()),
            (None, _) => "unavailable".to_owned(),
        }
    }

    pub fn bandwidth_label(&self, direction: &str) -> String {
        let (bps, age) = if direction == "to" {
            (self.to_peer_bandwidth, self.to_peer_age)
        } else {
            (self.from_peer_bandwidth, self.from_peer_age)
        };
        match bps {
            Some(value) => format!("{} ({})", format_bits_per_second(value), age.as_str()),
            None => "unavailable".to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UiSnapshot {
    pub screen: AppScreen,
    pub phase: RuntimePhase,
    pub local: LocalNodeSummary,
    pub peers: Vec<PeerSummary>,
    pub hardware: Option<HardwareSummaryView>,
    pub resources: ResourceManagerView,
    pub models: ModelStoreView,
    pub inference: crate::InferenceView,
    pub status_message: String,
    pub enrollment: EnrollmentProgress,
    pub can_create_invitation: bool,
}

impl UiSnapshot {
    pub fn starting(display_name: impl Into<String>) -> Self {
        Self {
            screen: AppScreen::FirstRun,
            phase: RuntimePhase::Starting,
            local: LocalNodeSummary {
                display_name: display_name.into(),
                node_id: None,
                mesh_id: None,
                listen_addr: None,
            },
            peers: Vec::new(),
            hardware: None,
            resources: ResourceManagerView::default(),
            models: ModelStoreView::default(),
            inference: crate::InferenceView::idle(),
            status_message: "Starting local runtime…".to_owned(),
            enrollment: EnrollmentProgress {
                steps: Vec::new(),
                current: String::new(),
                invitation_text: None,
                error: None,
                recovery: None,
                router_mapping_ok: None,
            },
            can_create_invitation: false,
        }
    }

    pub fn first_run(display_name: impl Into<String>) -> Self {
        let mut snapshot = Self::starting(display_name);
        snapshot.phase = RuntimePhase::AwaitingOnboarding;
        snapshot.status_message = "Create a mesh or enroll this PC.".to_owned();
        snapshot
    }
}

