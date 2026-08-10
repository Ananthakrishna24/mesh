use std::collections::HashMap;
use std::fmt::{Display, Formatter, Result as FmtResult};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use mesh_core::invite::{
    build_invite, candidates_from_proto, decode_invitation_text, encode_invitation_text,
};
use mesh_core::protocol::proto::ErrorCode;
use mesh_core::{
    AppScreen, CandidateKind, CapabilityReport, ConnectivityRecovery, CoreError, DEFAULT_CACHE_MAX_BYTES,
    DEFAULT_HOLD_LEASE_MS, DeploymentId, EndpointCandidate, EnrollmentId, EnrollmentProgress,
    GpuResourceAmount, HardwareSummaryView, InferencePhase, InferenceRequestSpec, LayerRange,
    LinkMeasurement, LocalIdentity, LocalNodeSummary, ManualForwardingGuide, MeshId, ModelCacheView,
    ModelDownloadProgress, ModelReference, NextTokenFeedback, NodeId, PeerRecord, PeerRecordOrigin,
    PeerSummary, PlacementPlan, ProviderAccessReport, ProviderAuthMode, RecoveryAction,
    ReplicaEndpointView, ReplicaHealth, RequestId, ReservationCommit, ReservationId,
    ReservationRelease, ReserveRequest, ResourceAmount, ResourceQuery, RuntimePhase, SamplingParams,
    StageAssignment, StageRole, StopReason, TokenResultEvent, UiCommand, UiSnapshot,
    filter_advertised_candidates, merge_peer_records, now_unix_ms, select_replica_route,
    sort_candidates_for_dial,
};
use mesh_hardware::discover_capabilities;
use mesh_inference::{
    load_mesh_tokenizer, LocalResourceManager, MeshTokenizer, ReserveOutcome, Sampler,
    SingleNodeEngine, StageActivation, StageHop, StageWorker,
};
use mesh_model::{
    DownloadProgressEvent, HuggingFaceProvider, PrepareResult, ProgressSink, ResolvedModel,
    build_complete_plan, build_stage_plan, cleanup_incomplete, prepare_plan,
};
use mesh_net::{
    ActivationFrame, EnrollmentHello, HOLE_PUNCH_WINDOW, IncomingPeer, MeshEndpoint,
    ReplicaStatusMessage, RouterMappingHandle, SessionCommand, SessionEvent, advertised_candidates,
    attempt_router_mapping, collect_local_candidates, complete_inviter_handshake,
    generate_node_certificate, new_attempt_id, perform_joiner_handshake, run_connected_session,
    send_udp_probes, start_at_after, wait_until_unix_ms, with_manual_candidate, with_peer_observed,
    with_router_mapping,
};
use mesh_store::{
    CredentialLookup, Store, StorePaths, delete_huggingface_token, huggingface_token_lookup,
    load_huggingface_token, save_huggingface_token,
};
use rand::RngCore;
use tokio::sync::{Mutex, broadcast, mpsc, oneshot, watch};
use tracing::{info, warn};

fn initial_provider_access_report() -> ProviderAccessReport {
    match huggingface_token_lookup() {
        CredentialLookup::Found => ProviderAccessReport {
            provider: mesh_core::PROVIDER_HUGGINGFACE.to_owned(),
            checked_at_unix_ms: 0,
            auth_mode: ProviderAuthMode::Saved,
            public_read: false,
            gated_read: false,
            status: mesh_core::ProviderAccessStatus::Unchecked,
            detail: "Saved Hugging Face token present; access not checked yet".to_owned(),
        },
        CredentialLookup::Missing => ProviderAccessReport::unchecked_huggingface(),
        CredentialLookup::StoreUnavailable => ProviderAccessReport {
            provider: mesh_core::PROVIDER_HUGGINGFACE.to_owned(),
            checked_at_unix_ms: 0,
            auth_mode: ProviderAuthMode::None,
            public_read: false,
            gated_read: false,
            status: mesh_core::ProviderAccessStatus::StoreUnavailable,
            detail: "Credential store unavailable".to_owned(),
        },
    }
}

const COMMAND_CAPACITY: usize = 64;
const EVENT_CAPACITY: usize = 64;
const INVITE_TTL_MS: i64 = 30 * 60 * 1000;
const PEER_UPDATE_COALESCE: Duration = Duration::from_secs(5);
const SELF_REFRESH_INTERVAL: Duration = Duration::from_secs(10 * 60);
const RESERVATION_SWEEP_INTERVAL: Duration = Duration::from_secs(5);
const CANDIDATE_STAGGER: Duration = Duration::from_millis(250);
const DIAL_ATTEMPT_TIMEOUT: Duration = Duration::from_secs(8);
#[derive(Debug)]
pub enum RuntimeError {
    Store(String),
    Net(String),
    Core(CoreError),
    CommandQueueClosed,
    AlreadyShutdown,
}

impl Display for RuntimeError {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::Store(message) | Self::Net(message) => write!(f, "{message}"),
            Self::Core(error) => write!(f, "{error}"),
            Self::CommandQueueClosed => write!(f, "runtime command queue closed"),
            Self::AlreadyShutdown => write!(f, "runtime already shut down"),
        }
    }
}

impl std::error::Error for RuntimeError {}

struct RuntimeState {
    snapshot: UiSnapshot,
    identity: Option<LocalIdentity>,
    candidates: Vec<EndpointCandidate>,
    peers: HashMap<NodeId, PeerSummary>,
    known_peers: Vec<PeerRecord>,
    hardware: Option<CapabilityReport>,
    pending_invitation: Option<String>,
    manual_public_address: String,
    last_join_failure_details: Vec<String>,
}

impl RuntimeState {
    fn new(display_name: String) -> Self {
        Self {
            snapshot: UiSnapshot::starting(display_name),
            identity: None,
            candidates: Vec::new(),
            peers: HashMap::new(),
            known_peers: Vec::new(),
            hardware: None,
            pending_invitation: None,
            manual_public_address: String::new(),
            last_join_failure_details: Vec::new(),
        }
    }

    fn set_status(&mut self, message: impl Into<String>) {
        self.snapshot.status_message = message.into();
    }

    fn push_step(&mut self, step: impl Into<String>) {
        let step = step.into();
        self.snapshot.enrollment.current = step.clone();
        self.snapshot.enrollment.steps.push(step);
    }

    fn set_error(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.snapshot.phase = RuntimePhase::Failed;
        self.snapshot.enrollment.error = Some(message.clone());
        self.snapshot.status_message = message;
    }

    fn apply_identity(&mut self, identity: LocalIdentity, listen_addr: SocketAddr) {
        self.snapshot.local = LocalNodeSummary {
            display_name: identity.display_name.clone(),
            node_id: Some(identity.node_id),
            mesh_id: Some(identity.mesh_id),
            listen_addr: Some(listen_addr),
        };
        self.identity = Some(identity);
        self.candidates = collect_local_candidates(listen_addr);
    }

    fn set_ready(&mut self, message: impl Into<String>) {
        self.snapshot.screen = AppScreen::Dashboard;
        self.snapshot.phase = RuntimePhase::Ready;
        self.snapshot.can_create_invitation = true;
        self.snapshot.status_message = message.into();
        self.snapshot.enrollment.error = None;
        self.snapshot.enrollment.recovery = None;
        self.rebuild_peer_summaries();
    }

    fn set_hardware(&mut self, report: CapabilityReport) {
        self.snapshot.hardware = Some(HardwareSummaryView::from_report(&report));
        self.hardware = Some(report);
    }

    fn rebuild_peer_summaries(&mut self) {
        let mut peers = self.peers.values().cloned().collect::<Vec<_>>();
        peers.sort_by(|left, right| left.display_name.cmp(&right.display_name));
        self.snapshot.peers = peers;
    }

    fn upsert_connected_peer(&mut self, peer: &PeerRecord, address: Option<SocketAddr>) {
        let previous = self.peers.get(&peer.node_id).cloned();
        self.peers.insert(
            peer.node_id,
            PeerSummary {
                node_id: peer.node_id,
                display_name: peer.display_name.clone(),
                connected: true,
                address,
                hardware_line: previous
                    .as_ref()
                    .and_then(|item| item.hardware_line.clone()),
                link: previous.as_ref().and_then(|item| item.link.clone()),
                replica_model_line: previous
                    .as_ref()
                    .and_then(|item| item.replica_model_line.clone()),
                replica_backend: previous
                    .as_ref()
                    .and_then(|item| item.replica_backend.clone()),
                replica_ready: previous.as_ref().is_some_and(|item| item.replica_ready),
                replica_healthy: previous.as_ref().is_some_and(|item| item.replica_healthy),
                replica_active_requests: previous
                    .as_ref()
                    .map(|item| item.replica_active_requests)
                    .unwrap_or(0),
                replica_max_concurrent_requests: previous
                    .as_ref()
                    .map(|item| item.replica_max_concurrent_requests)
                    .unwrap_or(0),
                replica_deployment_id: previous
                    .as_ref()
                    .and_then(|item| item.replica_deployment_id.clone()),
            },
        );
        if let Some(existing) = self
            .known_peers
            .iter_mut()
            .find(|known| known.node_id == peer.node_id)
        {
            *existing = peer.clone();
        } else {
            self.known_peers.push(peer.clone());
        }
        self.rebuild_peer_summaries();
    }

    fn apply_peer_capability(&mut self, peer_node_id: NodeId, report: &CapabilityReport) {
        if let Some(peer) = self.peers.get_mut(&peer_node_id) {
            peer.hardware_line = Some(report.summary_line());
            self.rebuild_peer_summaries();
        }
    }

    fn apply_link_measurement(&mut self, peer_node_id: NodeId, measurement: LinkMeasurement) {
        if let Some(peer) = self.peers.get_mut(&peer_node_id) {
            peer.link = Some(measurement);
            self.rebuild_peer_summaries();
        }
    }

    fn local_lan_address(&self) -> Option<SocketAddr> {
        self.candidates
            .iter()
            .find(|candidate| {
                candidate.kind == CandidateKind::LocalNetwork && !candidate.address.ip().is_loopback()
            })
            .map(|candidate| candidate.address)
            .or_else(|| self.snapshot.local.listen_addr)
    }

    fn build_no_direct_recovery(&self, details: Vec<String>) -> ConnectivityRecovery {
        let listen = self.snapshot.local.listen_addr;
        let local_address = self.local_lan_address();
        let port = listen.map(|addr| addr.port()).unwrap_or(0);
        ConnectivityRecovery {
            title: "No direct route".to_owned(),
            message: "The two routers did not allow a direct connection automatically.".to_owned(),
            primary: RecoveryAction::RetryAutomatic,
            secondary: Some(RecoveryAction::ShowManualSteps),
            technical_details: details,
            manual: Some(ManualForwardingGuide {
                local_udp_port: port,
                local_address,
                protocol: "UDP".to_owned(),
                public_address_input: self.manual_public_address.clone(),
                instructions: vec![
                    format!("Forward UDP port {port} on your router to this PC."),
                    "Use the local address shown below as the internal target.".to_owned(),
                    "Paste the resulting public IP:port and apply it, then create a new invitation on the other PC.".to_owned(),
                    "Provider-level CGNAT may still block manual forwarding.".to_owned(),
                ],
            }),
            show_manual: false,
            show_firewall_help: false,
            firewall_message: "This PC's firewall may be blocking the mesh connection. Allow the Mesh application to receive UDP connections, then try again.".to_owned(),
        }
    }

    fn self_peer_record(&self) -> Option<PeerRecord> {
        let identity = self.identity.as_ref()?;
        let mut record = PeerRecord::new(
            identity.node_id,
            identity.display_name.clone(),
            identity.certificate_der.clone(),
            advertised_candidates(&self.candidates),
        );
        record.origin = PeerRecordOrigin::LocalSelf;
        Some(record)
    }
}

#[derive(Clone, Debug)]
pub struct NodeHandle {
    commands: mpsc::Sender<UiCommand>,
    snapshots: watch::Receiver<UiSnapshot>,
    shutdown: broadcast::Sender<()>,
}

impl NodeHandle {
    pub fn snapshot(&self) -> UiSnapshot {
        self.snapshots.borrow().clone()
    }

    pub fn subscribe_snapshots(&self) -> watch::Receiver<UiSnapshot> {
        self.snapshots.clone()
    }

    pub async fn send(&self, command: UiCommand) -> Result<(), RuntimeError> {
        self.commands
            .send(command)
            .await
            .map_err(|_| RuntimeError::CommandQueueClosed)
    }

    pub fn try_send(&self, command: UiCommand) -> Result<(), RuntimeError> {
        self.commands
            .try_send(command)
            .map_err(|_| RuntimeError::CommandQueueClosed)
    }

    pub fn request_shutdown(&self) -> Result<(), RuntimeError> {
        match self.try_send(UiCommand::Shutdown) {
            Ok(()) => Ok(()),
            Err(RuntimeError::CommandQueueClosed) => {
                let _ = self.shutdown.send(());
                Err(RuntimeError::AlreadyShutdown)
            }
            Err(error) => Err(error),
        }
    }
}

enum RuntimeEvent {
    Incoming { incoming: IncomingPeer },
    PeerJoined {
        peer: PeerRecord,
        address: SocketAddr,
        session_commands: Option<mpsc::Sender<SessionCommand>>,
    },
    PeerFailed { message: String },
    Session {
        peer_node_id: NodeId,
        event: SessionEvent,
    },
    ModelAccess(Result<ProviderAccessReport, String>),
    ModelResolved(Result<ResolvedModel, String>),
    ModelPrepared(Result<PrepareResult, String>),
    ModelProgress(ModelDownloadProgress),
    ModelLoaded(Result<Box<SingleNodeEngine>, String>),
    PipelineStageLoaded(Result<(Box<LocalPipelineStage>, PrepareResult), String>),
    GenerationFinished {
        prompt: String,
        engine: Box<SingleNodeEngine>,
        result: Result<mesh_inference::GenerationOutput, String>,
        request_id: RequestId,
        remote_owner: Option<NodeId>,
    },
}

struct LiveSession {
    commands: mpsc::Sender<SessionCommand>,
}

struct PendingRemoteGeneration {
    peer_node_id: NodeId,
    request_id: RequestId,
    deployment_id: DeploymentId,
    prompt: String,
    token_ids: Vec<u32>,
    pipeline: bool,
}

struct ServingRemoteRequest {
    owner_node_id: NodeId,
    cancel: watch::Sender<bool>,
}

struct LocalPipelineStage {
    placement: PlacementPlan,
    assignment: StageAssignment,
    worker: StageWorker,
    model_line: String,
}

struct PipelineRequestState {
    #[allow(dead_code)]
    request_id: RequestId,
    deployment_id: DeploymentId,
    owner_node_id: NodeId,
    first_stage_node: NodeId,
    next_stage_node: Option<NodeId>,
    sampler: Option<Sampler>,
    sampling: SamplingParams,
    prompt_len: u32,
    stop_token_ids: Vec<u32>,
}

pub struct NodeRuntime {
    handle: NodeHandle,
    command_rx: mpsc::Receiver<UiCommand>,
    snapshot_tx: watch::Sender<UiSnapshot>,
    shutdown_tx: broadcast::Sender<()>,
    shutdown_rx: broadcast::Receiver<()>,
    event_tx: mpsc::Sender<RuntimeEvent>,
    event_rx: mpsc::Receiver<RuntimeEvent>,
    state: RuntimeState,
    store: Store,
    endpoint: Option<Arc<Mutex<MeshEndpoint>>>,
    paths: StorePaths,
    mapping: Option<RouterMappingHandle>,
    sessions: HashMap<NodeId, LiveSession>,
    resources: LocalResourceManager,
    pending_remote_reserves: HashMap<
        ReservationId,
        oneshot::Sender<Result<mesh_core::ReserveAccepted, mesh_core::ReserveRejected>>,
    >,
    peer_update_dirty: bool,
    last_peer_update: Option<tokio::time::Instant>,
    model_session_token: Option<String>,
    model_cancel: Option<watch::Sender<bool>>,
    resolved_model: Option<ResolvedModel>,
    last_prepare: Option<PrepareResult>,
    inference_engine: Option<SingleNodeEngine>,
    pipeline_stage: Option<LocalPipelineStage>,
    pipeline_requests: HashMap<RequestId, PipelineRequestState>,
    coordinator_tokenizer: Option<MeshTokenizer>,
    generation_cancel: Option<watch::Sender<bool>>,
    local_active_requests: u32,
    local_generation_request_id: Option<RequestId>,
    pending_remote_generation: Option<PendingRemoteGeneration>,
    serving_remote_requests: HashMap<RequestId, ServingRemoteRequest>,
}

impl NodeRuntime {
    pub fn create(display_name: impl Into<String>, paths: StorePaths) -> Result<Self, RuntimeError> {
        let display_name = display_name.into();
        let store =
            Store::open(paths.clone()).map_err(|error| RuntimeError::Store(error.to_string()))?;
        let mut state = RuntimeState::new(display_name);
        let hardware = discover_capabilities();
        state.set_hardware(hardware.clone());
        let restored = store
            .list_active_reservations()
            .map_err(|error| RuntimeError::Store(error.to_string()))?;
        let resources = LocalResourceManager::restore(
            mesh_core::ResourceCapacity::from_capability(&hardware),
            restored,
        );
        state.snapshot.resources = resources.view();
        let _ = cleanup_incomplete(&paths.model_cache_dir, now_unix_ms(), false);
        state.snapshot.models.cache = store
            .model_cache_view(
                paths.model_cache_dir.display().to_string(),
                DEFAULT_CACHE_MAX_BYTES,
            )
            .unwrap_or_default();
        state.snapshot.models.provider_access = initial_provider_access_report();
        let (command_tx, command_rx) = mpsc::channel(COMMAND_CAPACITY);
        let (snapshot_tx, snapshot_rx) = watch::channel(state.snapshot.clone());
        let (shutdown_tx, shutdown_rx) = broadcast::channel(EVENT_CAPACITY);
        let (event_tx, event_rx) = mpsc::channel(EVENT_CAPACITY);

        Ok(Self {
            handle: NodeHandle {
                commands: command_tx,
                snapshots: snapshot_rx,
                shutdown: shutdown_tx.clone(),
            },
            command_rx,
            snapshot_tx,
            shutdown_tx,
            shutdown_rx,
            event_tx,
            event_rx,
            state,
            store,
            endpoint: None,
            paths,
            mapping: None,
            sessions: HashMap::new(),
            resources,
            pending_remote_reserves: HashMap::new(),
            peer_update_dirty: false,
            last_peer_update: None,
            model_session_token: None,
            model_cancel: None,
            resolved_model: None,
            last_prepare: None,
            inference_engine: None,
            pipeline_stage: None,
            pipeline_requests: HashMap::new(),
            coordinator_tokenizer: None,
            generation_cancel: None,
            local_active_requests: 0,
            local_generation_request_id: None,
            pending_remote_generation: None,
            serving_remote_requests: HashMap::new(),
        })
    }

    pub fn handle(&self) -> NodeHandle {
        self.handle.clone()
    }

    pub fn data_dir(&self) -> &PathBuf {
        &self.paths.data_dir
    }

    pub async fn run(mut self) {
        info!(path = %self.paths.data_dir.display(), "node runtime started");
        if let Err(error) = self.bootstrap_existing().await {
            self.state.set_error(error.to_string());
            self.publish();
        } else if self.state.identity.is_none() {
            let name = self.state.snapshot.local.display_name.clone();
            let hardware = self.state.snapshot.hardware.clone();
            let resources = self.state.snapshot.resources.clone();
            let models = self.state.snapshot.models.clone();
            let inference = self.state.snapshot.inference.clone();
            self.state.snapshot = UiSnapshot::first_run(name);
            self.state.snapshot.hardware = hardware;
            self.state.snapshot.resources = resources;
            self.state.snapshot.models = models;
            self.state.snapshot.inference = inference;
            self.publish();
        }

        let mut peer_update_tick = tokio::time::interval(PEER_UPDATE_COALESCE);
        peer_update_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut self_refresh_tick = tokio::time::interval(SELF_REFRESH_INTERVAL);
        self_refresh_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut reservation_tick = tokio::time::interval(RESERVATION_SWEEP_INTERVAL);
        reservation_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                command = self.command_rx.recv() => {
                    match command {
                        Some(command) => {
                            if self.handle_command(command).await {
                                break;
                            }
                        }
                        None => break,
                    }
                }
                event = self.event_rx.recv() => {
                    match event {
                        Some(event) => self.handle_event(event).await,
                        None => break,
                    }
                }
                _ = peer_update_tick.tick() => {
                    if self.peer_update_dirty {
                        self.flush_peer_updates().await;
                    }
                }
                _ = self_refresh_tick.tick() => {
                    if self.state.identity.is_some() {
                        self.refresh_local_candidates().await;
                        self.mark_peer_update_dirty();
                    }
                }
                _ = reservation_tick.tick() => {
                    self.sweep_reservations();
                }
                shutdown = self.shutdown_rx.recv() => {
                    match shutdown {
                        Ok(()) | Err(broadcast::error::RecvError::Closed) => break,
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    }
                }
            }
        }

        let _ = self.shutdown_tx.send(());
        if let Some(mapping) = self.mapping.take() {
            mapping.delete().await;
        }
        self.sessions.clear();
        if let Some(endpoint) = self.endpoint.take() {
            endpoint.lock().await.close();
        }
        self.state.snapshot.phase = RuntimePhase::ShuttingDown;
        self.state.set_status("Shutting down…");
        self.publish();
        info!("node runtime stopped");
    }

    async fn bootstrap_existing(&mut self) -> Result<(), RuntimeError> {
        let Some(identity) = self
            .store
            .load_identity()
            .map_err(|error| RuntimeError::Store(error.to_string()))?
        else {
            return Ok(());
        };

        self.state.snapshot.phase = RuntimePhase::Preparing;
        self.state.push_step("Loaded this PC's identity");
        self.publish();

        let peers = self
            .store
            .list_peers()
            .map_err(|error| RuntimeError::Store(error.to_string()))?;
        self.state.known_peers = peers.clone();
        self.start_endpoint(identity).await?;
        self.state
            .set_ready("Local node restored. Reconnecting to known peers…");
        self.publish();

        for peer in peers {
            self.spawn_reconnect(peer);
        }
        Ok(())
    }

    async fn handle_command(&mut self, command: UiCommand) -> bool {
        match command {
            UiCommand::CreateMesh { display_name } => {
                if let Err(error) = self.create_mesh(display_name).await {
                    self.state.set_error(error.to_string());
                }
                self.publish();
                false
            }
            UiCommand::OpenEnrollment => {
                self.state.snapshot.screen = AppScreen::Enroll;
                self.state.snapshot.phase = RuntimePhase::AwaitingOnboarding;
                self.state.snapshot.enrollment = EnrollmentProgress {
                    steps: Vec::new(),
                    current: String::new(),
                    invitation_text: None,
                    error: None,
                    recovery: None,
                    router_mapping_ok: None,
                };
                self.state
                    .set_status("Paste an invitation from another PC.");
                self.publish();
                false
            }
            UiCommand::CancelEnrollment => {
                if self.state.identity.is_some() {
                    self.state.set_ready("Ready.");
                } else {
                    let name = self.state.snapshot.local.display_name.clone();
                    let hardware = self.state.snapshot.hardware.clone();
                    let resources = self.state.snapshot.resources.clone();
                    let models = self.state.snapshot.models.clone();
                    let inference = self.state.snapshot.inference.clone();
                    self.state.snapshot = UiSnapshot::first_run(name);
                    self.state.snapshot.hardware = hardware;
                    self.state.snapshot.resources = resources;
                    self.state.snapshot.models = models;
                    self.state.snapshot.inference = inference;
                }
                self.publish();
                false
            }
            UiCommand::SubmitInvitation { text } => {
                self.state.pending_invitation = Some(text.clone());
                if let Err(error) = self.join_with_invitation(text).await {
                    self.apply_join_failure(error);
                    self.state.snapshot.screen = AppScreen::Enroll;
                }
                self.publish();
                false
            }
            UiCommand::CreateInvitation => {
                if let Err(error) = self.create_invitation() {
                    self.state.set_error(error.to_string());
                }
                self.publish();
                false
            }
            UiCommand::ClearInvitation => {
                self.state.snapshot.enrollment.invitation_text = None;
                self.state.set_status("Invitation cleared.");
                self.publish();
                false
            }
            UiCommand::RefreshHardware => {
                let report = discover_capabilities();
                self.state.set_hardware(report.clone());
                self.resources.refresh_capacity(&report);
                self.publish_resources();
                self.state.set_status("Hardware report refreshed.");
                self.publish();
                false
            }
            UiCommand::RetryAutomaticConnectivity => {
                if let Some(text) = self.state.pending_invitation.clone() {
                    self.state.snapshot.enrollment.recovery = None;
                    self.state.snapshot.enrollment.error = None;
                    if let Err(error) = self.join_with_invitation(text).await {
                        self.apply_join_failure(error);
                    }
                } else {
                    self.refresh_local_candidates().await;
                    self.state
                        .set_status("Refreshed local connectivity candidates.");
                }
                self.publish();
                false
            }
            UiCommand::ShowManualForwarding => {
                if let Some(recovery) = self.state.snapshot.enrollment.recovery.as_mut() {
                    recovery.show_manual = true;
                    recovery.show_firewall_help = false;
                }
                self.publish();
                false
            }
            UiCommand::HideManualForwarding => {
                if let Some(recovery) = self.state.snapshot.enrollment.recovery.as_mut() {
                    recovery.show_manual = false;
                }
                self.publish();
                false
            }
            UiCommand::SetManualPublicAddress { address } => {
                self.state.manual_public_address = address.clone();
                if let Some(recovery) = self.state.snapshot.enrollment.recovery.as_mut() {
                    if let Some(manual) = recovery.manual.as_mut() {
                        manual.public_address_input = address;
                    }
                }
                self.publish();
                false
            }
            UiCommand::ApplyManualPublicAddress => {
                let text = self.state.manual_public_address.trim().to_owned();
                match text.parse::<SocketAddr>() {
                    Ok(address) => {
                        self.state.candidates =
                            with_manual_candidate(self.state.candidates.clone(), address);
                        self.mark_peer_update_dirty();
                        self.state
                            .set_status(format!("Manual public address set to {address}."));
                        if let Some(recovery) = self.state.snapshot.enrollment.recovery.as_mut() {
                            recovery.message = "Manual address saved. Create a new invitation on a connected PC and try again.".to_owned();
                        }
                    }
                    Err(_) => {
                        self.state
                            .set_status("Manual address must look like 203.0.113.10:4433.");
                    }
                }
                self.publish();
                false
            }
            UiCommand::ShowFirewallHelp => {
                if let Some(recovery) = self.state.snapshot.enrollment.recovery.as_mut() {
                    recovery.show_firewall_help = true;
                } else {
                    let mut recovery = self
                        .state
                        .build_no_direct_recovery(self.state.last_join_failure_details.clone());
                    recovery.show_firewall_help = true;
                    self.state.snapshot.enrollment.recovery = Some(recovery);
                }
                self.publish();
                false
            }
            UiCommand::HideFirewallHelp => {
                if let Some(recovery) = self.state.snapshot.enrollment.recovery.as_mut() {
                    recovery.show_firewall_help = false;
                }
                self.publish();
                false
            }
            UiCommand::RunLocalReservationProbe => {
                if let Err(error) = self.run_local_reservation_probe() {
                    self.state.set_status(error);
                }
                self.publish();
                false
            }
            UiCommand::ReleaseAllLocalReservations => {
                self.release_all_local_reservations();
                self.state
                    .set_status("Released all local resource reservations.");
                self.publish();
                false
            }
            UiCommand::SelectModel { reference } => {
                self.select_model(reference);
                self.publish();
                false
            }
            UiCommand::RefreshProviderAccess => {
                self.spawn_provider_access_probe(None);
                self.publish();
                false
            }
            UiCommand::SaveHuggingFaceToken { token } => {
                self.save_huggingface_token_command(token);
                self.publish();
                false
            }
            UiCommand::DeleteHuggingFaceToken => {
                self.delete_huggingface_token_command();
                self.publish();
                false
            }
            UiCommand::ProbeSelectedModel => {
                self.spawn_model_resolve();
                self.publish();
                false
            }
            UiCommand::PrepareSelectedModel => {
                self.spawn_model_prepare();
                self.publish();
                false
            }
            UiCommand::CancelModelWork => {
                self.cancel_model_work();
                self.publish();
                false
            }
            UiCommand::ClearModelCache => {
                self.clear_model_cache();
                self.publish();
                false
            }
            UiCommand::LoadSelectedModel => {
                self.spawn_model_load();
                self.publish();
                false
            }
            UiCommand::LoadPipelineStage {
                deployment_id,
                model_line,
                num_layers,
                stage_index,
                role,
                layer_start,
                layer_end,
                node_ids,
            } => {
                self.spawn_pipeline_stage_load(
                    deployment_id,
                    model_line,
                    num_layers,
                    stage_index,
                    role,
                    layer_start,
                    layer_end,
                    node_ids,
                );
                self.publish();
                false
            }
            UiCommand::UnloadModel => {
                self.unload_model();
                self.publish();
                false
            }
            UiCommand::Generate {
                prompt,
                max_new_tokens,
                temperature,
                seed,
            } => {
                self.spawn_generation(prompt, max_new_tokens, temperature, seed);
                self.publish();
                false
            }
            UiCommand::CancelGeneration => {
                self.cancel_generation();
                self.publish();
                false
            }
            UiCommand::Shutdown => true,
        }
    }

    fn apply_join_failure(&mut self, error: RuntimeError) {
        let message = error.to_string();
        let mut details = self.state.last_join_failure_details.clone();
        details.push(message.clone());
        self.state.last_join_failure_details = details.clone();
        self.state.set_error(
            "The two routers did not allow a direct connection automatically.".to_owned(),
        );
        self.state.snapshot.enrollment.recovery =
            Some(self.state.build_no_direct_recovery(details));
    }

    async fn handle_event(&mut self, event: RuntimeEvent) {
        match event {
            RuntimeEvent::Incoming { incoming } => {
                if let Err(error) = self.handle_incoming(incoming).await {
                    warn!(%error, "incoming handshake failed");
                }
            }
            RuntimeEvent::PeerJoined {
                peer,
                address,
                session_commands,
            } => {
                if let Some(commands) = session_commands {
                    self.sessions
                        .entry(peer.node_id)
                        .or_insert(LiveSession { commands });
                } else if self.sessions.contains_key(&peer.node_id) {
                    return;
                }
                self.state.upsert_connected_peer(&peer, Some(address));
                if let Err(error) = self.store.upsert_peer(&peer) {
                    warn!(%error, "failed to persist joined peer");
                }
                self.state
                    .set_status(format!("Connected to {}.", peer.display_name));
                if self.state.snapshot.screen != AppScreen::Dashboard {
                    self.state.set_ready("This PC is ready.");
                } else {
                    self.state.snapshot.phase = RuntimePhase::Ready;
                    self.state.snapshot.can_create_invitation = true;
                }
                self.mark_peer_update_dirty();
                self.maybe_introduce_unconnected_peers(peer.node_id).await;
                self.announce_replica_status_to(peer.node_id);
                self.publish();
            }
            RuntimeEvent::PeerFailed { message } => {
                warn!(%message, "peer task failed");
            }
            RuntimeEvent::Session {
                peer_node_id,
                event,
            } => {
                self.handle_session_event(peer_node_id, event).await;
            }
            RuntimeEvent::ModelAccess(result) => {
                self.on_model_access(result);
                self.publish();
            }
            RuntimeEvent::ModelResolved(result) => {
                self.on_model_resolved(result);
                self.publish();
            }
            RuntimeEvent::ModelPrepared(result) => {
                self.on_model_prepared(result);
                self.publish();
            }
            RuntimeEvent::ModelProgress(progress) => {
                self.state.snapshot.models.progress = Some(progress);
                self.publish();
            }
            RuntimeEvent::ModelLoaded(result) => {
                self.on_model_loaded(result);
                self.publish();
            }
            RuntimeEvent::PipelineStageLoaded(result) => {
                self.on_pipeline_stage_loaded(result);
                self.publish();
            }
            RuntimeEvent::GenerationFinished {
                prompt,
                engine,
                result,
                request_id,
                remote_owner,
            } => {
                self.on_generation_finished(prompt, engine, result, request_id, remote_owner)
                    .await;
                self.publish();
            }

        }
    }

    async fn handle_session_event(&mut self, peer_node_id: NodeId, event: SessionEvent) {
        match event {
            SessionEvent::Capability {
                peer_node_id,
                report,
            } => {
                self.state.apply_peer_capability(peer_node_id, &report);
                self.publish();
            }
            SessionEvent::Link {
                peer_node_id,
                measurement,
            } => {
                self.state.apply_link_measurement(peer_node_id, measurement);
                self.publish();
            }
            SessionEvent::PeerUpdate { from_peer, peers } => {
                self.apply_peer_update(from_peer, peers).await;
            }
            SessionEvent::IntroductionOffer {
                from_peer,
                target_node_id,
                attempt_id,
                start_at_unix_ms,
                observed_address,
            } => {
                self.handle_introduction_offer(
                    from_peer,
                    target_node_id,
                    attempt_id,
                    start_at_unix_ms,
                    observed_address,
                )
                .await;
            }
            SessionEvent::IntroductionReady {
                peer_node_id: target,
                peer_observed,
                self_observed,
                start_at_unix_ms,
                ..
            } => {
                let _ = self_observed;
                if let Some(peer) = self
                    .state
                    .known_peers
                    .iter()
                    .find(|peer| peer.node_id == target)
                    .cloned()
                {
                    let mut peer = peer;
                    peer.candidates = with_peer_observed(
                        peer.candidates,
                        peer_observed,
                        peer_node_id,
                    );
                    self.spawn_holepunch_dial(peer, peer_observed, start_at_unix_ms);
                }
            }
            SessionEvent::PeerObserve {
                observed_node_id,
                address,
                from_peer,
                ..
            } => {
                if let Some(peer) = self
                    .state
                    .known_peers
                    .iter_mut()
                    .find(|peer| peer.node_id == observed_node_id)
                {
                    peer.candidates =
                        with_peer_observed(peer.candidates.clone(), address, from_peer);
                    let _ = self.store.upsert_peer(peer);
                }
            }
            SessionEvent::ResourceQuery {
                from_peer,
                message_id,
                query,
            } => {
                self.handle_resource_query(from_peer, message_id, query)
                    .await;
            }
            SessionEvent::ResourceOffer { from_peer, offer } => {
                info!(
                    %from_peer,
                    can_satisfy = offer.can_satisfy,
                    "received resource offer"
                );
            }
            SessionEvent::ReserveRequest {
                from_peer,
                message_id,
                request,
            } => {
                self.handle_reserve_request(from_peer, message_id, request)
                    .await;
            }
            SessionEvent::ReserveAccepted {
                from_peer,
                accepted,
            } => {
                if let Some(tx) = self
                    .pending_remote_reserves
                    .remove(&accepted.reservation_id)
                {
                    let _ = tx.send(Ok(accepted));
                } else {
                    info!(%from_peer, "ignored unexpected reserve accepted");
                }
            }
            SessionEvent::ReserveRejected {
                from_peer,
                rejected,
            } => {
                if let Some(tx) = self
                    .pending_remote_reserves
                    .remove(&rejected.reservation_id)
                {
                    let _ = tx.send(Err(rejected));
                } else {
                    info!(%from_peer, "ignored unexpected reserve rejected");
                }
            }
            SessionEvent::ReservationCommit { from_peer, commit } => {
                self.handle_reservation_commit(from_peer, commit);
            }
            SessionEvent::ReservationRelease { from_peer, release } => {
                self.handle_reservation_release(from_peer, release);
            }
            SessionEvent::ReplicaStatus { from_peer, status } => {
                self.on_replica_status(from_peer, status);
                self.publish();
            }
            SessionEvent::InferenceRequest { from_peer, request } => {
                self.on_remote_inference_request(from_peer, request).await;
                self.publish();
            }
            SessionEvent::TokenResult { from_peer, event } => {
                let _ = from_peer;
                self.on_remote_token(event);
                self.publish();
            }
            SessionEvent::CancelRequest {
                from_peer,
                deployment_id,
                request_id,
                reason,
            } => {
                let _ = (from_peer, reason);
                if let Some(serving) = self.serving_remote_requests.get(&request_id) {
                    let _ = serving.cancel.send(true);
                }
                if self.local_generation_request_id == Some(request_id) {
                    if let Some(cancel) = self.generation_cancel.as_ref() {
                        let _ = cancel.send(true);
                    }
                }
                self.cancel_pipeline_request(deployment_id, request_id);
                self.publish();
            }
            SessionEvent::NextTokenFeedback {
                from_peer,
                feedback,
            } => {
                self.on_pipeline_next_token_feedback(from_peer, feedback);
                self.publish();
            }
            SessionEvent::Activation { from_peer, frame } => {
                self.on_pipeline_activation(from_peer, frame);
                self.publish();
            }
            SessionEvent::Failed {
                peer_node_id,
                message,
            } => {
                warn!(%peer_node_id, %message, "peer session failed");
                self.sessions.remove(&peer_node_id);
                self.release_owner_reservations(peer_node_id);
                self.fail_pending_remote_for_peer(peer_node_id, message.clone());
                self.cancel_serving_for_peer(peer_node_id);
                self.cancel_pipeline_for_peer(peer_node_id);
                if let Some(peer) = self.state.peers.get_mut(&peer_node_id) {
                    peer.connected = false;
                    peer.replica_ready = false;
                    peer.replica_healthy = false;
                    self.state.rebuild_peer_summaries();
                }
                self.refresh_replica_views();
                self.publish();
            }
        }
    }

    async fn apply_peer_update(&mut self, from_peer: NodeId, peers: Vec<PeerRecord>) {
        let Some(local_id) = self.state.identity.as_ref().map(|identity| identity.node_id) else {
            return;
        };
        let now = now_unix_ms();
        let mut changed = false;
        for incoming in peers {
            if incoming.node_id == local_id {
                continue;
            }
            let existing = self
                .state
                .known_peers
                .iter()
                .find(|peer| peer.node_id == incoming.node_id)
                .cloned();
            let from_direct_subject = incoming.node_id == from_peer;
            match merge_peer_records(
                existing.as_ref(),
                &incoming,
                local_id,
                now,
                from_direct_subject,
            ) {
                Ok(merged) => {
                    if let Some(slot) = self
                        .state
                        .known_peers
                        .iter_mut()
                        .find(|peer| peer.node_id == merged.node_id)
                    {
                        if *slot != merged {
                            *slot = merged.clone();
                            changed = true;
                        }
                    } else {
                        self.state.known_peers.push(merged.clone());
                        changed = true;
                    }
                    if let Err(error) = self.store.upsert_peer(&merged) {
                        warn!(%error, "failed to persist peer update");
                    }
                    if !self.sessions.contains_key(&merged.node_id)
                        && !self
                            .state
                            .peers
                            .get(&merged.node_id)
                            .is_some_and(|peer| peer.connected)
                    {
                        self.spawn_reconnect(merged);
                    }
                }
                Err(error) => warn!(%error, "rejected peer update"),
            }
        }
        if changed {
            self.state.rebuild_peer_summaries();
            self.publish();
        }
    }

    async fn handle_introduction_offer(
        &mut self,
        from_peer: NodeId,
        target_node_id: NodeId,
        attempt_id: [u8; 16],
        start_at_unix_ms: i64,
        observed_address: SocketAddr,
    ) {
        let Some(local_id) = self.state.identity.as_ref().map(|identity| identity.node_id) else {
            return;
        };
        if target_node_id != local_id {
            // Relay introduction to the target if we are connected to it.
            if let Some(session) = self.sessions.get(&target_node_id) {
                let self_observed = self
                    .state
                    .peers
                    .get(&target_node_id)
                    .and_then(|peer| peer.address)
                    .unwrap_or(observed_address);
                let _ = session
                    .commands
                    .send(SessionCommand::SendIntroductionReady {
                        attempt_id,
                        peer_node_id: from_peer,
                        peer_observed: observed_address,
                        self_observed,
                        start_at_unix_ms,
                    })
                    .await;
            }
            return;
        }

        if let Some(peer) = self
            .state
            .known_peers
            .iter()
            .find(|peer| peer.node_id == from_peer)
            .cloned()
        {
            let mut peer = peer;
            peer.candidates =
                with_peer_observed(peer.candidates, observed_address, from_peer);
            self.spawn_holepunch_dial(peer, observed_address, start_at_unix_ms);
        }
    }

    async fn maybe_introduce_unconnected_peers(&mut self, newly_connected: NodeId) {
        let connected: Vec<NodeId> = self.sessions.keys().copied().collect();
        if connected.len() < 2 {
            return;
        }
        let Some(new_addr) = self
            .state
            .peers
            .get(&newly_connected)
            .and_then(|peer| peer.address)
        else {
            return;
        };

        for other in connected.into_iter().filter(|id| *id != newly_connected) {
            if self
                .state
                .peers
                .get(&other)
                .is_some_and(|peer| peer.connected)
            {
                // Introduce newly_connected and other to each other through this node.
                let Some(other_addr) = self.state.peers.get(&other).and_then(|peer| peer.address)
                else {
                    continue;
                };
                let attempt_id = new_attempt_id();
                let start_at = start_at_after(Duration::from_millis(300));
                if let Some(session) = self.sessions.get(&newly_connected) {
                    let _ = session
                        .commands
                        .send(SessionCommand::SendIntroductionOffer {
                            target_node_id: other,
                            attempt_id,
                            start_at_unix_ms: start_at,
                            observed_address: other_addr,
                        })
                        .await;
                }
                if let Some(session) = self.sessions.get(&other) {
                    let _ = session
                        .commands
                        .send(SessionCommand::SendIntroductionOffer {
                            target_node_id: newly_connected,
                            attempt_id,
                            start_at_unix_ms: start_at,
                            observed_address: new_addr,
                        })
                        .await;
                }
            }
        }
    }

    async fn handle_incoming(&mut self, incoming: IncomingPeer) -> Result<(), RuntimeError> {
        let identity = self
            .state
            .identity
            .clone()
            .ok_or_else(|| RuntimeError::Store("missing local identity".to_owned()))?;
        let local_candidates = advertised_candidates(&self.state.candidates);
        let known_peers = self
            .state
            .known_peers
            .iter()
            .cloned()
            .filter(|peer| peer.node_id != identity.node_id)
            .collect::<Vec<_>>();
        let remote_address = incoming.remote_address;
        let connection = incoming.connection;
        let hardware = self
            .state
            .hardware
            .clone()
            .unwrap_or_else(discover_capabilities);

        let (peer, send, recv) = complete_inviter_handshake(
            &connection,
            &identity,
            &local_candidates,
            &known_peers,
            incoming.peer_certificate_der,
            |hello, peer| accept_enrollment(&mut self.store, hello, peer),
        )
        .await
        .map_err(|error| RuntimeError::Net(error.to_string()))?;

        let mut peer = peer;
        peer.candidates = with_peer_observed(peer.candidates, remote_address, identity.node_id);
        peer.last_successful_address = Some(remote_address);
        peer.last_seen_unix_ms = Some(now_unix_ms());
        if let Err(error) = self.store.upsert_peer(&peer) {
            warn!(%error, "failed to persist incoming peer");
        }

        self.state.upsert_connected_peer(&peer, Some(remote_address));
        self.state
            .set_status(format!("Connected to {}.", peer.display_name));
        self.state.snapshot.phase = RuntimePhase::Ready;
        self.state.snapshot.can_create_invitation = true;
        self.mark_peer_update_dirty();
        self.publish();

        self.spawn_session(identity, peer.node_id, connection, send, recv, hardware);
        self.maybe_introduce_unconnected_peers(peer.node_id).await;
        Ok(())
    }

    async fn create_mesh(&mut self, display_name: String) -> Result<(), RuntimeError> {
        if self.state.identity.is_some() {
            return Err(RuntimeError::Store(
                "this PC is already part of a mesh".to_owned(),
            ));
        }

        self.state.snapshot.phase = RuntimePhase::Preparing;
        self.state.snapshot.enrollment.error = None;
        self.state.snapshot.enrollment.recovery = None;
        self.state.snapshot.enrollment.steps.clear();
        self.state.push_step("Creating this PC's identity");
        self.publish();

        let certificate =
            generate_node_certificate().map_err(|error| RuntimeError::Net(error.to_string()))?;
        let identity = self
            .store
            .create_mesh_identity(
                normalize_display_name(display_name),
                certificate.certificate_der,
                certificate.private_key_der,
            )
            .map_err(|error| RuntimeError::Store(error.to_string()))?;

        self.state.push_step("Created this PC's identity");
        self.start_endpoint(identity).await?;
        self.state.push_step("Opened the local connection port");
        if self.state.snapshot.enrollment.router_mapping_ok == Some(true) {
            self.state.push_step("Router connection prepared automatically");
        }
        self.state
            .set_ready("This PC is ready. Add another PC to enroll a peer.");
        Ok(())
    }

    async fn join_with_invitation(&mut self, text: String) -> Result<(), RuntimeError> {
        if self.state.identity.is_some()
            && self.state.snapshot.phase == RuntimePhase::Ready
            && self.state.snapshot.screen == AppScreen::Dashboard
        {
            return Err(RuntimeError::Store(
                "this PC is already part of a mesh".to_owned(),
            ));
        }

        // Allow retry after a failed join by resetting identity only when not enrolled.
        if self.state.snapshot.phase == RuntimePhase::Failed {
            if let Some(endpoint) = self.endpoint.take() {
                endpoint.lock().await.close();
            }
            if let Some(mapping) = self.mapping.take() {
                mapping.delete().await;
            }
            self.sessions.clear();
            self.state.identity = None;
            self.state.candidates.clear();
            self.state.known_peers.clear();
            self.state.peers.clear();
        }

        if self.state.identity.is_some() {
            return Err(RuntimeError::Store(
                "this PC is already part of a mesh".to_owned(),
            ));
        }

        self.state.snapshot.screen = AppScreen::Enroll;
        self.state.snapshot.phase = RuntimePhase::Preparing;
        self.state.snapshot.enrollment.error = None;
        self.state.snapshot.enrollment.recovery = None;
        self.state.snapshot.enrollment.steps.clear();
        self.state.last_join_failure_details.clear();
        self.state.push_step("Reading invitation");
        self.publish();

        let invite = decode_invitation_text(&text).map_err(RuntimeError::Core)?;
        let mesh_id = MeshId::from_slice(&invite.mesh_id).map_err(RuntimeError::Core)?;
        let inviter_node_id =
            NodeId::from_slice(&invite.inviter_node_id).map_err(RuntimeError::Core)?;
        let enrollment_id =
            EnrollmentId::from_slice(&invite.enrollment_id).map_err(RuntimeError::Core)?;
        let mut secret = [0u8; 32];
        secret.copy_from_slice(&invite.enrollment_secret);
        let mut candidates = candidates_from_proto(&invite.candidates).map_err(RuntimeError::Core)?;
        sort_candidates_for_dial(&mut candidates);

        self.state.push_step("Creating this PC's identity");
        self.publish();

        let certificate =
            generate_node_certificate().map_err(|error| RuntimeError::Net(error.to_string()))?;
        let identity = self
            .store
            .create_joining_identity(
                normalize_display_name(self.state.snapshot.local.display_name.clone()),
                mesh_id,
                certificate.certificate_der,
                certificate.private_key_der,
            )
            .map_err(|error| RuntimeError::Store(error.to_string()))?;

        self.state.push_step("Created this PC's identity");
        self.start_endpoint(identity.clone()).await?;
        self.state.push_step("Opened the local connection port");
        if self.state.snapshot.enrollment.router_mapping_ok == Some(true) {
            self.state.push_step("Router connection prepared automatically");
        }
        self.state.snapshot.phase = RuntimePhase::Connecting;
        self.state
            .push_step(format!("Connecting to {}", invite.inviter_name));
        self.publish();

        let endpoint = self
            .endpoint
            .clone()
            .ok_or_else(|| RuntimeError::Net("endpoint missing".to_owned()))?;
        let local_candidates = advertised_candidates(&self.state.candidates);
        let hardware = self
            .state
            .hardware
            .clone()
            .unwrap_or_else(discover_capabilities);

        let mut last_error = RuntimeError::Net("no invitation candidates succeeded".to_owned());
        let mut details = Vec::new();
        for (index, candidate) in candidates.into_iter().enumerate() {
            if index > 0 {
                tokio::time::sleep(CANDIDATE_STAGGER).await;
            }
            let connect_result = {
                let mut guard = endpoint.lock().await;
                tokio::time::timeout(
                    DIAL_ATTEMPT_TIMEOUT,
                    guard.connect(candidate.address, inviter_node_id),
                )
                .await
            };
            match connect_result {
                Ok(Ok(peer_connection)) => {
                    match perform_joiner_handshake(
                        &peer_connection.connection,
                        &identity,
                        &local_candidates,
                        Some(enrollment_id),
                        Some(secret),
                        inviter_node_id,
                    )
                    .await
                    {
                        Ok((welcome, send, recv)) => {
                            self.store
                                .accept_enrollment_snapshot(
                                    &welcome.responder,
                                    &welcome.known_peers,
                                )
                                .map_err(|error| RuntimeError::Store(error.to_string()))?;
                            self.state.known_peers = welcome.known_peers.clone();
                            self.state.upsert_connected_peer(
                                &welcome.responder,
                                Some(peer_connection.remote_address),
                            );
                            self.state.push_step("Connected to the existing PC");
                            self.state.push_step("Received the known PC list");
                            self.state.set_ready("This PC is ready.");
                            self.mark_peer_update_dirty();

                            self.spawn_session(
                                identity.clone(),
                                welcome.responder.node_id,
                                peer_connection.connection,
                                send,
                                recv,
                                hardware.clone(),
                            );

                            for peer in welcome.known_peers {
                                if peer.node_id != welcome.responder.node_id {
                                    self.spawn_reconnect(peer);
                                }
                            }
                            return Ok(());
                        }
                        Err(error) => {
                            details.push(format!(
                                "{} via {}: {error}",
                                candidate.kind.priority(),
                                candidate.address
                            ));
                            last_error = RuntimeError::Net(error.to_string());
                        }
                    }
                }
                Ok(Err(error)) => {
                    details.push(format!("{}: {error}", candidate.address));
                    last_error = RuntimeError::Net(error.to_string());
                }
                Err(_) => {
                    details.push(format!("{}: dial timed out", candidate.address));
                    last_error = RuntimeError::Net(format!(
                        "dial timed out for {}",
                        candidate.address
                    ));
                }
            }
        }

        self.state.last_join_failure_details = details;
        Err(last_error)
    }

    fn create_invitation(&mut self) -> Result<(), RuntimeError> {
        let identity = self
            .state
            .identity
            .clone()
            .ok_or_else(|| RuntimeError::Store("create a mesh before inviting peers".to_owned()))?;
        let candidates = advertised_candidates(&self.state.candidates);
        if candidates.is_empty() {
            return Err(RuntimeError::Net(
                "no local candidates available for invitation".to_owned(),
            ));
        }

        let enrollment_id = EnrollmentId::new();
        let mut secret = [0u8; 32];
        rand::rng().fill_bytes(&mut secret);
        let expires_at_unix_ms = now_unix_ms() + INVITE_TTL_MS;
        self.store
            .create_invitation(enrollment_id, &secret, expires_at_unix_ms)
            .map_err(|error| RuntimeError::Store(error.to_string()))?;

        let invite = build_invite(
            identity.mesh_id,
            identity.node_id,
            identity.display_name,
            enrollment_id,
            secret,
            expires_at_unix_ms,
            &candidates,
        )
        .map_err(RuntimeError::Core)?;
        let text = encode_invitation_text(&invite).map_err(RuntimeError::Core)?;
        self.state.snapshot.enrollment.invitation_text = Some(text);
        self.state
            .set_status("Invitation ready. Copy it to the new PC.");
        Ok(())
    }

    async fn start_endpoint(&mut self, identity: LocalIdentity) -> Result<(), RuntimeError> {
        let socket = std::net::UdpSocket::bind(SocketAddr::from(([0, 0, 0, 0], 0)))
            .map_err(|error| RuntimeError::Net(error.to_string()))?;
        let listen_addr = socket
            .local_addr()
            .map_err(|error| RuntimeError::Net(error.to_string()))?;
        let endpoint = MeshEndpoint::from_udp_socket(identity.clone(), socket)
            .map_err(|error| RuntimeError::Net(error.to_string()))?;

        self.state.apply_identity(identity, listen_addr);
        self.state.candidates = collect_local_candidates(listen_addr);

        match attempt_router_mapping(listen_addr.port()).await {
            Ok(handle) => {
                self.state.candidates =
                    with_router_mapping(self.state.candidates.clone(), handle.result());
                self.state.snapshot.enrollment.router_mapping_ok = Some(true);
                self.spawn_mapping_renew_loop();
                self.mapping = Some(handle);
            }
            Err(error) => {
                warn!(%error, "router mapping unavailable");
                self.state.snapshot.enrollment.router_mapping_ok = Some(false);
            }
        }

        let endpoint = Arc::new(Mutex::new(endpoint));
        self.spawn_accept_loop(endpoint.clone());
        self.endpoint = Some(endpoint);
        Ok(())
    }

    fn spawn_mapping_renew_loop(&self) {
        // Renew is driven from the owned handle on shutdown and via candidate refresh ticks.
        // Full async ownership of RouterMappingHandle stays on the runtime for delete safety.
    }

    async fn refresh_local_candidates(&mut self) {
        let Some(listen) = self.state.snapshot.local.listen_addr else {
            return;
        };
        let mut candidates = collect_local_candidates(listen);
        candidates.extend(
            self.state
                .candidates
                .iter()
                .filter(|candidate| {
                    matches!(
                        candidate.kind,
                        CandidateKind::RouterMapping | CandidateKind::Manual | CandidateKind::PeerObserved
                    )
                })
                .cloned(),
        );
        // Dedup by address keeping higher priority.
        sort_candidates_for_dial(&mut candidates);
        let mut seen = std::collections::BTreeSet::new();
        candidates.retain(|candidate| seen.insert(candidate.address));
        self.state.candidates = filter_advertised_candidates(&candidates, now_unix_ms());
        if self.mapping.is_none() {
            if let Ok(handle) = attempt_router_mapping(listen.port()).await {
                self.state.candidates =
                    with_router_mapping(self.state.candidates.clone(), handle.result());
                self.state.snapshot.enrollment.router_mapping_ok = Some(true);
                self.mapping = Some(handle);
            }
        } else if let Some(mapping) = self.mapping.as_mut() {
            match mapping.renew().await {
                Ok(result) => {
                    self.state.candidates = with_router_mapping(
                        self.state
                            .candidates
                            .iter()
                            .filter(|item| item.kind != CandidateKind::RouterMapping)
                            .cloned()
                            .collect(),
                        &result,
                    );
                    self.state.snapshot.enrollment.router_mapping_ok = Some(true);
                }
                Err(error) => {
                    warn!(%error, "mapping renew failed");
                    // Contract: retry once immediately, then after 30s, then drop.
                    match mapping.renew().await {
                        Ok(result) => {
                            self.state.candidates = with_router_mapping(
                                self.state
                                    .candidates
                                    .iter()
                                    .filter(|item| item.kind != CandidateKind::RouterMapping)
                                    .cloned()
                                    .collect(),
                                &result,
                            );
                        }
                        Err(_) => {
                            tokio::time::sleep(Duration::from_secs(30)).await;
                            if let Ok(result) = mapping.renew().await {
                                self.state.candidates = with_router_mapping(
                                    self.state
                                        .candidates
                                        .iter()
                                        .filter(|item| item.kind != CandidateKind::RouterMapping)
                                        .cloned()
                                        .collect(),
                                    &result,
                                );
                            } else if let Some(handle) = self.mapping.take() {
                                handle.delete().await;
                                self.state
                                    .candidates
                                    .retain(|item| item.kind != CandidateKind::RouterMapping);
                                self.state.snapshot.enrollment.router_mapping_ok = Some(false);
                            }
                        }
                    }
                }
            }
        }
    }

    fn spawn_accept_loop(&self, endpoint: Arc<Mutex<MeshEndpoint>>) {
        let event_tx = self.event_tx.clone();
        let mut shutdown_rx = self.shutdown_tx.subscribe();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => break,
                    accepted = async {
                        let guard = endpoint.lock().await;
                        // accept only needs &self; clone endpoint out of lock quickly.
                        let endpoint = guard.clone();
                        drop(guard);
                        endpoint.accept().await
                    } => {
                        match accepted {
                            Ok(incoming) => {
                                if event_tx
                                    .send(RuntimeEvent::Incoming { incoming })
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            Err(error) => {
                                warn!(%error, "accept loop ended");
                                break;
                            }
                        }
                    }
                }
            }
        });
    }

    fn spawn_session(
        &mut self,
        identity: LocalIdentity,
        peer_node_id: NodeId,
        connection: quinn::Connection,
        send: quinn::SendStream,
        recv: quinn::RecvStream,
        hardware: CapabilityReport,
    ) {
        if self.sessions.contains_key(&peer_node_id) {
            connection.close(0u32.into(), b"duplicate");
            return;
        }
        let event_tx = self.event_tx.clone();
        let (command_tx, command_rx) = mpsc::channel(EVENT_CAPACITY);
        self.sessions.insert(
            peer_node_id,
            LiveSession {
                commands: command_tx,
            },
        );

        tokio::spawn(async move {
            let (session_tx, mut session_rx) = mpsc::channel(EVENT_CAPACITY);
            let forward = tokio::spawn({
                let event_tx = event_tx.clone();
                async move {
                    while let Some(event) = session_rx.recv().await {
                        if event_tx
                            .send(RuntimeEvent::Session {
                                peer_node_id,
                                event,
                            })
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            });
            run_connected_session(
                identity,
                peer_node_id,
                connection,
                send,
                recv,
                hardware,
                session_tx,
                command_rx,
            )
            .await;
            let _ = forward.await;
            let _ = event_tx
                .send(RuntimeEvent::Session {
                    peer_node_id,
                    event: SessionEvent::Failed {
                        peer_node_id,
                        message: "session ended".to_owned(),
                    },
                })
                .await;
        });
    }

    fn spawn_reconnect(&self, peer: PeerRecord) {
        if self.sessions.contains_key(&peer.node_id) {
            return;
        }
        let Some(identity) = self.state.identity.clone() else {
            return;
        };
        let Some(endpoint) = self.endpoint.clone() else {
            return;
        };
        let local_candidates = advertised_candidates(&self.state.candidates);
        let hardware = self
            .state
            .hardware
            .clone()
            .unwrap_or_else(discover_capabilities);
        let event_tx = self.event_tx.clone();
        let mut candidates = peer.candidates.clone();
        sort_candidates_for_dial(&mut candidates);

        tokio::spawn(async move {
            for (index, candidate) in candidates.into_iter().enumerate() {
                if index > 0 {
                    tokio::time::sleep(CANDIDATE_STAGGER).await;
                }
                let connected = {
                    let mut guard = endpoint.lock().await;
                    tokio::time::timeout(
                        DIAL_ATTEMPT_TIMEOUT,
                        guard.connect(candidate.address, peer.node_id),
                    )
                    .await
                };
                let Ok(Ok(connection)) = connected else {
                    continue;
                };
                match perform_joiner_handshake(
                    &connection.connection,
                    &identity,
                    &local_candidates,
                    None,
                    None,
                    peer.node_id,
                )
                .await
                {
                    Ok((welcome, send, recv)) => {
                        let remote = connection.remote_address;
                        let keep = connection.connection;
                        let (session_tx, mut session_rx) = mpsc::channel(EVENT_CAPACITY);
                        let (command_tx, command_rx) = mpsc::channel(EVENT_CAPACITY);
                        let _ = event_tx
                            .send(RuntimeEvent::PeerJoined {
                                peer: welcome.responder.clone(),
                                address: remote,
                                session_commands: Some(command_tx),
                            })
                            .await;
                        let forward_tx = event_tx.clone();
                        let peer_node_id = welcome.responder.node_id;
                        let forward = tokio::spawn(async move {
                            while let Some(event) = session_rx.recv().await {
                                if forward_tx
                                    .send(RuntimeEvent::Session {
                                        peer_node_id,
                                        event,
                                    })
                                    .await
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        });
                        run_connected_session(
                            identity,
                            welcome.responder.node_id,
                            keep,
                            send,
                            recv,
                            hardware,
                            session_tx,
                            command_rx,
                        )
                        .await;
                        let _ = forward.await;
                        return;
                    }
                    Err(error) => warn!(%error, "reconnect handshake failed"),
                }
            }
            let _ = event_tx
                .send(RuntimeEvent::PeerFailed {
                    message: format!("failed to reconnect to {}", peer.display_name),
                })
                .await;
        });
    }

    fn spawn_holepunch_dial(
        &self,
        peer: PeerRecord,
        observed: SocketAddr,
        start_at_unix_ms: i64,
    ) {
        if self.sessions.contains_key(&peer.node_id) {
            return;
        }
        let Some(identity) = self.state.identity.clone() else {
            return;
        };
        let Some(endpoint) = self.endpoint.clone() else {
            return;
        };
        let local_candidates = advertised_candidates(&self.state.candidates);
        let hardware = self
            .state
            .hardware
            .clone()
            .unwrap_or_else(discover_capabilities);
        let event_tx = self.event_tx.clone();

        tokio::spawn(async move {
            wait_until_unix_ms(start_at_unix_ms).await;
            send_udp_probes(&[observed], HOLE_PUNCH_WINDOW).await;
            let connected = {
                let mut guard = endpoint.lock().await;
                tokio::time::timeout(DIAL_ATTEMPT_TIMEOUT, guard.connect(observed, peer.node_id))
                    .await
            };
            let Ok(Ok(connection)) = connected else {
                return;
            };
            match perform_joiner_handshake(
                &connection.connection,
                &identity,
                &local_candidates,
                None,
                None,
                peer.node_id,
            )
            .await
            {
                Ok((welcome, send, recv)) => {
                    let remote = connection.remote_address;
                    let keep = connection.connection;
                    let (session_tx, mut session_rx) = mpsc::channel(EVENT_CAPACITY);
                    let (command_tx, command_rx) = mpsc::channel(EVENT_CAPACITY);
                    let _ = event_tx
                        .send(RuntimeEvent::PeerJoined {
                            peer: welcome.responder.clone(),
                            address: remote,
                            session_commands: Some(command_tx),
                        })
                        .await;
                    let forward_tx = event_tx.clone();
                    let peer_node_id = welcome.responder.node_id;
                    let forward = tokio::spawn(async move {
                        while let Some(event) = session_rx.recv().await {
                            if forward_tx
                                .send(RuntimeEvent::Session {
                                    peer_node_id,
                                    event,
                                })
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                    });
                    run_connected_session(
                        identity,
                        welcome.responder.node_id,
                        keep,
                        send,
                        recv,
                        hardware,
                        session_tx,
                        command_rx,
                    )
                    .await;
                    let _ = forward.await;
                }
                Err(error) => warn!(%error, "hole-punch handshake failed"),
            }
        });
    }

    fn mark_peer_update_dirty(&mut self) {
        self.peer_update_dirty = true;
    }

    async fn flush_peer_updates(&mut self) {
        self.peer_update_dirty = false;
        self.last_peer_update = Some(tokio::time::Instant::now());
        let Some(self_record) = self.state.self_peer_record() else {
            return;
        };
        let mut peers = vec![self_record];
        for peer in &self.state.known_peers {
            if peers.len() >= 64 {
                break;
            }
            let mut advertised = peer.clone();
            advertised.candidates = advertised_candidates(&peer.candidates);
            peers.push(advertised);
        }
        let sessions: Vec<_> = self
            .sessions
            .iter()
            .map(|(id, session)| (*id, session.commands.clone()))
            .collect();
        for (_id, commands) in sessions {
            let _ = commands
                .send(SessionCommand::SendPeerUpdate {
                    peers: peers.clone(),
                })
                .await;
        }
    }

    fn select_model(&mut self, reference: ModelReference) {
        self.resolved_model = None;
        self.state.snapshot.models.selected_reference = Some(reference.clone());
        self.state.snapshot.models.selected_model = Some(reference.repository.clone());
        self.state.snapshot.models.resolved_identity = None;
        self.state.snapshot.models.error = None;
        self.state.snapshot.models.progress = None;
        self.state.snapshot.models.last_prepare_summary = None;
        self.state.snapshot.models.status_line =
            format!("Selected {}@{}", reference.repository, reference.revision_hint);
        self.state
            .set_status(format!("Selected model {}.", reference.repository));
    }

    fn save_huggingface_token_command(&mut self, token: String) {
        let trimmed = token.trim().to_owned();
        if trimmed.is_empty() {
            self.state.snapshot.models.error = Some("Token cannot be empty.".to_owned());
            self.state.snapshot.models.status_line = "Hugging Face token rejected".to_owned();
            return;
        }
        match save_huggingface_token(&trimmed) {
            Ok(()) => {
                self.model_session_token = None;
                self.state.snapshot.models.error = None;
                self.state.snapshot.models.status_line = "Saved Hugging Face token".to_owned();
                self.state.set_status("Saved Hugging Face token.");
                self.spawn_provider_access_probe(Some(trimmed));
            }
            Err(error) => {
                self.model_session_token = Some(trimmed.clone());
                self.state.snapshot.models.error = Some(format!(
                    "Credential store unavailable ({error}). Using session-only token."
                ));
                self.state.snapshot.models.status_line =
                    "Using session-only Hugging Face token".to_owned();
                self.state
                    .set_status("Credential store unavailable; token kept for this session only.");
                self.spawn_provider_access_probe(Some(trimmed));
            }
        }
    }

    fn delete_huggingface_token_command(&mut self) {
        self.model_session_token = None;
        match delete_huggingface_token() {
            Ok(_) => {
                self.state.snapshot.models.error = None;
                self.state.snapshot.models.status_line = "Deleted Hugging Face token".to_owned();
                self.state.set_status("Deleted Hugging Face token.");
            }
            Err(error) => {
                self.state.snapshot.models.error = Some(error.to_string());
                self.state.snapshot.models.status_line = "Failed to delete token".to_owned();
            }
        }
        self.spawn_provider_access_probe(None);
    }

    fn cancel_model_work(&mut self) {
        if let Some(cancel) = self.model_cancel.take() {
            let _ = cancel.send(true);
        }
        self.state.snapshot.models.busy = false;
        self.state.snapshot.models.progress = None;
        self.state.snapshot.models.status_line = "Model work cancelled".to_owned();
        self.state.set_status("Cancelled model work.");
    }

    fn clear_model_cache(&mut self) {
        let entries = self.store.list_model_cache_entries().unwrap_or_default();
        let mut removed = 0u32;
        for entry in entries {
            if entry.reference_count > 0 || entry.pinned {
                continue;
            }
            let path = self.paths.model_cache_dir.join(&entry.relative_path);
            let _ = std::fs::remove_file(path);
            if self.store.delete_model_cache_entry(&entry.entry_id).is_ok() {
                removed = removed.saturating_add(1);
            }
        }
        let _ = cleanup_incomplete(&self.paths.model_cache_dir, now_unix_ms(), true);
        self.refresh_model_cache_view();
        self.state.snapshot.models.status_line = format!("Cleared {removed} cache entries");
        self.state
            .set_status(format!("Cleared {removed} unreferenced model cache entries."));
    }

    fn refresh_model_cache_view(&mut self) {
        self.state.snapshot.models.cache = self
            .store
            .model_cache_view(
                self.paths.model_cache_dir.display().to_string(),
                DEFAULT_CACHE_MAX_BYTES,
            )
            .unwrap_or_else(|_| ModelCacheView {
                root: self.paths.model_cache_dir.display().to_string(),
                ..ModelCacheView::default()
            });
    }

    fn current_provider_token(&self) -> (Option<String>, ProviderAuthMode) {
        if let Some(token) = &self.model_session_token {
            return (Some(token.clone()), ProviderAuthMode::Session);
        }
        match load_huggingface_token() {
            Ok(Some(token)) => (Some(token), ProviderAuthMode::Saved),
            Ok(None) => (None, ProviderAuthMode::None),
            Err(_) => (None, ProviderAuthMode::None),
        }
    }

    fn build_provider(&self) -> Result<HuggingFaceProvider, String> {
        let (token, auth_mode) = self.current_provider_token();
        let hf_cache = self.paths.cache_dir.join("hf-hub");
        HuggingFaceProvider::new(token, auth_mode, hf_cache).map_err(|error| error.to_string())
    }

    fn spawn_provider_access_probe(&mut self, override_token: Option<String>) {
        let reference = self
            .state
            .snapshot
            .models
            .selected_reference
            .clone()
            .unwrap_or_else(ModelReference::qwen3_4b);
        let event_tx = self.event_tx.clone();
        let hf_cache = self.paths.cache_dir.join("hf-hub");
        let (token, auth_mode) = if let Some(token) = override_token {
            let mode = if self.model_session_token.as_ref() == Some(&token) {
                ProviderAuthMode::Session
            } else {
                ProviderAuthMode::Saved
            };
            (Some(token), mode)
        } else {
            self.current_provider_token()
        };
        self.state.snapshot.models.busy = true;
        self.state.snapshot.models.status_line = "Checking Hugging Face access…".to_owned();
        tokio::spawn(async move {
            let result = async {
                let provider = HuggingFaceProvider::new(token, auth_mode, hf_cache)
                    .map_err(|error| error.to_string())?;
                provider
                    .probe_access(&reference)
                    .await
                    .map_err(|error| error.to_string())
            }
            .await;
            let _ = event_tx.send(RuntimeEvent::ModelAccess(result)).await;
        });
    }

    fn spawn_model_resolve(&mut self) {
        let Some(reference) = self.state.snapshot.models.selected_reference.clone() else {
            self.state.snapshot.models.error = Some("Select a model first.".to_owned());
            self.state.snapshot.models.status_line = "No model selected".to_owned();
            return;
        };
        let provider = match self.build_provider() {
            Ok(provider) => provider,
            Err(error) => {
                self.state.snapshot.models.error = Some(error);
                return;
            }
        };
        let event_tx = self.event_tx.clone();
        let (cancel_tx, mut cancel_rx) = watch::channel(false);
        self.model_cancel = Some(cancel_tx);
        self.state.snapshot.models.busy = true;
        self.state.snapshot.models.error = None;
        self.state.snapshot.models.progress = None;
        self.state.snapshot.models.status_line =
            format!("Resolving {}…", reference.repository);
        self.state
            .set_status(format!("Resolving model {}…", reference.repository));
        tokio::spawn(async move {
            let work = provider.resolve(&reference);
            tokio::select! {
                result = work => {
                    let mapped = result.map_err(|error| error.to_string());
                    let _ = event_tx.send(RuntimeEvent::ModelResolved(mapped)).await;
                }
                _ = cancel_rx.changed() => {
                    if *cancel_rx.borrow() {
                        let _ = event_tx
                            .send(RuntimeEvent::ModelResolved(Err(
                                "model resolve cancelled".to_owned(),
                            )))
                            .await;
                    }
                }
            }
        });
    }

    fn spawn_model_prepare(&mut self) {
        let Some(resolved) = self.resolved_model.clone() else {
            self.state.snapshot.models.error =
                Some("Probe/resolve the model before prepare.".to_owned());
            self.state.snapshot.models.status_line = "Model not resolved".to_owned();
            return;
        };
        if !self
            .state
            .snapshot
            .models
            .provider_access
            .status
            .is_ready()
            && self.state.snapshot.models.provider_access.public_read == false
            && self.current_provider_token().0.is_none()
        {
            // still allow public models; prepare will fail truthfully if denied
        }
        let provider = match self.build_provider() {
            Ok(provider) => provider,
            Err(error) => {
                self.state.snapshot.models.error = Some(error);
                return;
            }
        };
        let plan = match build_complete_plan(DeploymentId::new().to_string(), &resolved) {
            Ok(plan) => plan,
            Err(error) => {
                self.state.snapshot.models.error = Some(error.to_string());
                return;
            }
        };
        let existing = self.store.list_model_cache_entries().unwrap_or_default();
        let cache_root = self.paths.model_cache_dir.clone();
        let event_tx = self.event_tx.clone();
        let (cancel_tx, mut cancel_rx) = watch::channel(false);
        self.model_cancel = Some(cancel_tx);
        self.state.snapshot.models.busy = true;
        self.state.snapshot.models.error = None;
        self.state.snapshot.models.progress = None;
        self.state.snapshot.models.status_line = format!(
            "Preparing {} ({} bytes planned)…",
            resolved.identity.repository, plan.disk_bytes_required
        );
        self.state.set_status("Preparing model artifacts…");

        struct ChannelProgress {
            tx: mpsc::Sender<RuntimeEvent>,
        }
        impl ProgressSink for ChannelProgress {
            fn on_progress(&mut self, event: DownloadProgressEvent) {
                let _ = self.tx.try_send(RuntimeEvent::ModelProgress(event.progress));
            }
        }

        tokio::spawn(async move {
            let mut progress = ChannelProgress {
                tx: event_tx.clone(),
            };
            let work = prepare_plan(
                &provider,
                &resolved,
                &plan,
                &cache_root,
                &existing,
                &mut progress,
            );
            tokio::select! {
                result = work => {
                    let mapped = result.map_err(|error| error.to_string());
                    let _ = event_tx.send(RuntimeEvent::ModelPrepared(mapped)).await;
                }
                _ = cancel_rx.changed() => {
                    if *cancel_rx.borrow() {
                        let _ = event_tx
                            .send(RuntimeEvent::ModelPrepared(Err(
                                "model prepare cancelled".to_owned(),
                            )))
                            .await;
                    }
                }
            }
        });
    }

    fn on_model_access(&mut self, result: Result<ProviderAccessReport, String>) {
        self.state.snapshot.models.busy = false;
        match result {
            Ok(report) => {
                self.state.snapshot.models.provider_access = report.clone();
                self.state.snapshot.models.error = None;
                self.state.snapshot.models.status_line = report.detail.clone();
                self.state.set_status(report.detail);
            }
            Err(error) => {
                self.state.snapshot.models.error = Some(error.clone());
                self.state.snapshot.models.status_line = "Provider access check failed".to_owned();
                self.state.set_status(error);
            }
        }
    }

    fn on_model_resolved(&mut self, result: Result<ResolvedModel, String>) {
        self.model_cancel = None;
        self.state.snapshot.models.busy = false;
        match result {
            Ok(resolved) => {
                let record = mesh_core::ModelManifestRecord {
                    cache_key: resolved.manifest.cache_key(),
                    provider: resolved.identity.provider.clone(),
                    repository: resolved.identity.repository.clone(),
                    revision: resolved.identity.revision.clone(),
                    adapter_id: resolved.manifest.adapter_id.clone(),
                    adapter_version: resolved.manifest.adapter_version.clone(),
                    model_format: resolved.identity.model_format,
                    quantization: resolved.identity.quantization.clone(),
                    manifest_hash: resolved.identity.manifest_hash.clone(),
                    canonical_bytes: mesh_model::canonical_manifest_bytes(&resolved.manifest)
                        .unwrap_or_default(),
                    created_at_unix_ms: now_unix_ms(),
                };
                if let Err(error) = self.store.upsert_model_manifest(&record) {
                    warn!(%error, "failed to persist model manifest");
                }
                self.state.snapshot.models.resolved_identity = Some(resolved.identity.clone());
                self.state.snapshot.models.selected_model =
                    Some(resolved.identity.summary_line());
                self.state.snapshot.models.error = None;
                self.state.snapshot.models.status_line = format!(
                    "Resolved {} ({} tensors)",
                    resolved.identity.summary_line(),
                    resolved.manifest.tensors.len()
                );
                self.state.set_status(format!(
                    "Resolved model {}.",
                    resolved.identity.summary_line()
                ));
                self.resolved_model = Some(resolved);
            }
            Err(error) => {
                self.state.snapshot.models.error = Some(error.clone());
                self.state.snapshot.models.status_line = "Model resolve failed".to_owned();
                self.state.set_status(error);
            }
        }
    }

    fn ensure_coordinator_tokenizer(&mut self) {
        if self.coordinator_tokenizer.is_some() {
            return;
        }
        let Some(resolved) = self.resolved_model.as_ref() else {
            return;
        };
        match load_mesh_tokenizer(&self.paths.model_cache_dir, resolved, None) {
            Ok(tokenizer) => self.coordinator_tokenizer = Some(tokenizer),
            Err(error) => warn!(%error, "failed to load coordinator tokenizer"),
        }
    }

    fn on_model_prepared(&mut self, result: Result<PrepareResult, String>) {
        self.model_cancel = None;
        self.state.snapshot.models.busy = false;
        self.state.snapshot.models.progress = None;
        match result {
            Ok(prepared) => {
                for entry in &prepared.cache_entries {
                    if let Err(error) = self.store.upsert_model_cache_entry(entry) {
                        warn!(%error, "failed to persist cache entry");
                    }
                }
                self.refresh_model_cache_view();
                self.state.snapshot.models.resolved_identity = Some(prepared.identity.clone());
                self.state.snapshot.models.last_prepare_summary = Some(prepared.summary.clone());
                self.state.snapshot.models.error = None;
                self.state.snapshot.models.status_line = prepared.summary.clone();
                self.state.set_status(prepared.summary.clone());
                self.last_prepare = Some(prepared);
                self.ensure_coordinator_tokenizer();
            }
            Err(error) => {
                self.state.snapshot.models.error = Some(error.clone());
                self.state.snapshot.models.status_line = "Model prepare failed".to_owned();
                self.state.set_status(error);
            }
        }
    }

    fn spawn_model_load(&mut self) {
        let Some(resolved) = self.resolved_model.clone() else {
            self.state.snapshot.inference.error =
                Some("Probe/resolve the model before load.".to_owned());
            self.state.snapshot.inference.status_line = "Model not resolved".to_owned();
            return;
        };
        let Some(prepared) = self.last_prepare.clone() else {
            self.state.snapshot.inference.error =
                Some("Prepare downloads before load.".to_owned());
            self.state.snapshot.inference.status_line = "Model not prepared".to_owned();
            return;
        };
        if self.state.snapshot.inference.busy {
            return;
        }
        let cache_root = self.paths.model_cache_dir.clone();
        let event_tx = self.event_tx.clone();
        self.state.snapshot.inference.busy = true;
        self.state.snapshot.inference.phase = Some(InferencePhase::Loading);
        self.state.snapshot.inference.error = None;
        self.state.snapshot.inference.status_line = "Loading model…".to_owned();
        self.state.set_status("Loading model into compute backend…");
        tokio::task::spawn_blocking(move || {
            let deployment_id = DeploymentId::new();
            let prefer_cuda = true;
            let result = SingleNodeEngine::load(
                deployment_id,
                &resolved,
                &prepared,
                &cache_root,
                prefer_cuda,
                None,
            )
            .map(Box::new)
            .map_err(|error| error.to_string());
            let _ = event_tx.blocking_send(RuntimeEvent::ModelLoaded(result));
        });
    }

    fn on_model_loaded(&mut self, result: Result<Box<SingleNodeEngine>, String>) {
        self.state.snapshot.inference.busy = false;
        match result {
            Ok(engine) => {
                let mut engine = *engine;
                if let Err(error) = engine.warmup() {
                    self.state.snapshot.inference.phase = Some(InferencePhase::Failed);
                    self.state.snapshot.inference.error = Some(error.to_string());
                    self.state.snapshot.inference.status_line = "Warm-up failed".to_owned();
                    self.state.set_status(error.to_string());
                    return;
                }
                self.state.snapshot.inference = engine.view("", "", None);
                self.state
                    .set_status(format!("Model ready on {}.", engine.backend.as_str()));
                self.coordinator_tokenizer = Some(engine.tokenizer().clone());
                self.inference_engine = Some(engine);
                self.local_active_requests = 0;
                self.refresh_replica_views();
                self.broadcast_replica_status();
            }
            Err(error) => {
                self.state.snapshot.inference.phase = Some(InferencePhase::Failed);
                self.state.snapshot.inference.error = Some(error.clone());
                self.state.snapshot.inference.status_line = "Model load failed".to_owned();
                self.state.set_status(error);
                self.refresh_replica_views();
            }
        }
    }

    fn unload_model(&mut self) {
        if let Some(cancel) = self.generation_cancel.take() {
            let _ = cancel.send(true);
        }
        for serving in self.serving_remote_requests.values() {
            let _ = serving.cancel.send(true);
        }
        self.serving_remote_requests.clear();
        self.pending_remote_generation = None;
        self.local_active_requests = 0;
        self.local_generation_request_id = None;
        self.inference_engine = None;
        if let Some(mut stage) = self.pipeline_stage.take() {
            for request_id in self.pipeline_requests.keys().copied().collect::<Vec<_>>() {
                stage.worker.cancel(request_id);
            }
        }
        self.pipeline_requests.clear();
        self.state.snapshot.inference = mesh_core::InferenceView::idle();
        self.refresh_replica_views();
        self.broadcast_replica_status();
        self.state.set_status("Unloaded model.");
    }

    fn spawn_generation(
        &mut self,
        prompt: String,
        max_new_tokens: u32,
        temperature: f32,
        seed: u64,
    ) {
        if self.state.snapshot.inference.busy || self.pending_remote_generation.is_some() {
            return;
        }
        let params = SamplingParams {
            temperature,
            top_k: if temperature == 0.0 {
                0
            } else {
                mesh_core::DEFAULT_TOP_K
            },
            top_p: if temperature == 0.0 {
                1.0
            } else {
                mesh_core::DEFAULT_TOP_P
            },
            repetition_penalty: mesh_core::DEFAULT_REPETITION_PENALTY,
            seed,
            max_new_tokens: max_new_tokens.max(1),
        };

        if self.try_spawn_pipeline_generation(prompt.clone(), params).is_some() {
            return;
        }

        self.refresh_replica_views();
        let preferred_model = self
            .inference_engine
            .as_ref()
            .map(|engine| engine.model_line.clone())
            .or_else(|| self.state.snapshot.inference.model_line.clone());
        let Some(route) = select_replica_route(
            self.state.snapshot.inference.replicas.iter(),
            preferred_model.as_deref(),
        )
        .cloned() else {
            self.state.snapshot.inference.error =
                Some("No ready replica with free capacity.".to_owned());
            self.state.snapshot.inference.status_line = "No replica available".to_owned();
            return;
        };

        if route.local {
            self.spawn_local_generation(prompt, params);
            return;
        }

        let Ok(peer_node_id) = NodeId::parse_hex(&route.node_id) else {
            self.state.snapshot.inference.error = Some("Invalid replica node id.".to_owned());
            return;
        };
        let Ok(deployment_id) = DeploymentId::parse_hex(&route.deployment_id) else {
            self.state.snapshot.inference.error = Some("Invalid replica deployment id.".to_owned());
            return;
        };
        let Some(session) = self.sessions.get(&peer_node_id) else {
            self.state.snapshot.inference.error = Some("Replica peer is not connected.".to_owned());
            return;
        };

        let Some(tokenizer) = self.coordinator_tokenizer.as_ref() else {
            self.state.snapshot.inference.error = Some(
                "Prepare/load a model once so this node can tokenize remote requests.".to_owned(),
            );
            self.state.snapshot.inference.status_line = "Local tokenizer required".to_owned();
            return;
        };
        let token_ids = match tokenizer.encode_chat(None, &prompt) {
            Ok(ids) => ids,
            Err(error) => {
                self.state.snapshot.inference.error = Some(error.to_string());
                return;
            }
        };
        let request_id = RequestId::new();
        let request = InferenceRequestSpec {
            deployment_id,
            request_id,
            input_token_ids: token_ids,
            sampling: params,
            stop_token_ids: Vec::new(),
            return_logprobs: false,
        };
        if session
            .commands
            .try_send(SessionCommand::SendInferenceRequest { request })
            .is_err()
        {
            self.state.snapshot.inference.error =
                Some("Failed to send inference request to replica.".to_owned());
            return;
        }

        self.pending_remote_generation = Some(PendingRemoteGeneration {
            peer_node_id,
            request_id,
            deployment_id,
            prompt: prompt.clone(),
            token_ids: Vec::new(),
            pipeline: false,
        });
        if let Some(peer) = self.state.peers.get_mut(&peer_node_id) {
            peer.replica_active_requests = peer.replica_active_requests.saturating_add(1);
            peer.replica_ready = peer.replica_active_requests < peer.replica_max_concurrent_requests;
            self.state.rebuild_peer_summaries();
        }
        self.state.snapshot.inference.busy = true;
        self.state.snapshot.inference.phase = Some(InferencePhase::Generating);
        self.state.snapshot.inference.prompt = prompt;
        self.state.snapshot.inference.output_text.clear();
        self.state.snapshot.inference.error = None;
        self.state.snapshot.inference.stop_reason = None;
        self.state.snapshot.inference.generated_tokens = 0;
        self.state.snapshot.inference.last_token_id = None;
        self.state.snapshot.inference.routed_node_id = Some(route.node_id);
        self.state.snapshot.inference.model_line = Some(route.model_line);
        self.state.snapshot.inference.backend = Some(route.backend);
        self.state.snapshot.inference.deployment_id = Some(deployment_id.to_string());
        self.state.snapshot.inference.status_line =
            format!("Generating on remote replica {}…", route.display_name);
        self.state
            .set_status(format!("Routed generation to {}.", route.display_name));
        self.refresh_replica_views();
    }

    fn spawn_local_generation(&mut self, prompt: String, params: SamplingParams) {
        let max_concurrent = self
            .inference_engine
            .as_ref()
            .map(|engine| engine.max_concurrent_requests())
            .unwrap_or(1);
        if self.local_active_requests >= max_concurrent {
            self.state.snapshot.inference.error =
                Some("Local replica has no free execution slots.".to_owned());
            return;
        }
        let Some(mut engine) = self.inference_engine.take() else {
            self.state.snapshot.inference.error = Some("Load a model before generating.".to_owned());
            self.state.snapshot.inference.status_line = "No model loaded".to_owned();
            return;
        };
        let request_id = RequestId::new();
        let local_node = self
            .state
            .identity
            .as_ref()
            .map(|identity| identity.node_id.to_string());
        let (cancel_tx, cancel_rx) = watch::channel(false);
        self.generation_cancel = Some(cancel_tx);
        self.local_generation_request_id = Some(request_id);
        self.local_active_requests = self.local_active_requests.saturating_add(1);
        self.state.snapshot.inference.busy = true;
        self.state.snapshot.inference.phase = Some(InferencePhase::Generating);
        self.state.snapshot.inference.prompt = prompt.clone();
        self.state.snapshot.inference.output_text.clear();
        self.state.snapshot.inference.error = None;
        self.state.snapshot.inference.stop_reason = None;
        self.state.snapshot.inference.routed_node_id = local_node;
        self.state.snapshot.inference.status_line = "Generating on local replica…".to_owned();
        self.state.set_status("Generating tokens…");
        self.refresh_replica_views();
        self.broadcast_replica_status();
        let event_tx = self.event_tx.clone();
        tokio::task::spawn_blocking(move || {
            let result = engine
                .generate_with_request(&prompt, params, request_id, |_| {}, || !*cancel_rx.borrow())
                .map_err(|error| error.to_string());
            let _ = event_tx.blocking_send(RuntimeEvent::GenerationFinished {
                prompt,
                engine: Box::new(engine),
                result,
                request_id,
                remote_owner: None,
            });
        });
    }

    fn cancel_generation(&mut self) {
        if let Some(cancel) = self.generation_cancel.take() {
            let _ = cancel.send(true);
        }
        if let Some(pending) = self.pending_remote_generation.as_ref() {
            if let Some(session) = self.sessions.get(&pending.peer_node_id) {
                let _ = session.commands.try_send(SessionCommand::SendCancelRequest {
                    deployment_id: pending.deployment_id,
                    request_id: pending.request_id,
                    reason: "cancelled by user".to_owned(),
                });
            }
        }
        self.state.snapshot.inference.status_line = "Cancelling generation…".to_owned();
    }

    async fn on_generation_finished(
        &mut self,
        prompt: String,
        engine: Box<SingleNodeEngine>,
        result: Result<mesh_inference::GenerationOutput, String>,
        request_id: RequestId,
        remote_owner: Option<NodeId>,
    ) {
        self.generation_cancel = None;
        self.local_generation_request_id = None;
        self.local_active_requests = self.local_active_requests.saturating_sub(1);
        self.serving_remote_requests.remove(&request_id);
        self.inference_engine = Some(*engine);

        if let Some(owner) = remote_owner {
            match &result {
                Ok(output) => {
                    if let Some(session) = self.sessions.get(&owner) {
                        for event in &output.tokens {
                            let _ = session
                                .commands
                                .try_send(SessionCommand::SendTokenResult { event: event.clone() });
                        }
                        if output.tokens.last().is_none_or(|event| !event.is_last) {
                            let final_event = TokenResultEvent {
                                deployment_id: self
                                    .inference_engine
                                    .as_ref()
                                    .map(|engine| engine.deployment_id)
                                    .unwrap_or_else(DeploymentId::new),
                                request_id,
                                token_id: output
                                    .tokens
                                    .last()
                                    .map(|event| event.token_id)
                                    .unwrap_or(0),
                                token_index: output.tokens.len() as u32,
                                is_last: true,
                                stop_reason: Some(output.stop_reason),
                                sequence_length: output
                                    .tokens
                                    .last()
                                    .map(|event| event.sequence_length)
                                    .unwrap_or(0),
                            };
                            let _ = session
                                .commands
                                .try_send(SessionCommand::SendTokenResult { event: final_event });
                        }
                    }
                }
                Err(error) => {
                    warn!(%owner, %error, "remote generation failed");
                    if let Some(session) = self.sessions.get(&owner) {
                        let event = TokenResultEvent {
                            deployment_id: self
                                .inference_engine
                                .as_ref()
                                .map(|engine| engine.deployment_id)
                                .unwrap_or_else(DeploymentId::new),
                            request_id,
                            token_id: 0,
                            token_index: 0,
                            is_last: true,
                            stop_reason: Some(StopReason::Error),
                            sequence_length: 0,
                        };
                        let _ = session
                            .commands
                            .try_send(SessionCommand::SendTokenResult { event });
                    }
                }
            }
            self.refresh_replica_views();
            self.broadcast_replica_status();
            return;
        }

        self.state.snapshot.inference.busy = false;
        self.state.snapshot.inference.prompt = prompt;
        match result {
            Ok(output) => {
                self.state.snapshot.inference.phase = Some(InferencePhase::Ready);
                self.state.snapshot.inference.output_text = output.text;
                self.state.snapshot.inference.generated_tokens = output.tokens.len() as u32;
                self.state.snapshot.inference.stop_reason =
                    Some(output.stop_reason.as_str().to_owned());
                self.state.snapshot.inference.last_token_id =
                    output.tokens.last().map(|token| token.token_id);
                self.state.snapshot.inference.error = None;
                self.state.snapshot.inference.backend = self
                    .inference_engine
                    .as_ref()
                    .map(|engine| engine.backend.as_str().to_owned());
                self.state.snapshot.inference.model_line = self
                    .inference_engine
                    .as_ref()
                    .map(|engine| engine.model_line.clone());
                self.state.snapshot.inference.deployment_id = self
                    .inference_engine
                    .as_ref()
                    .map(|engine| engine.deployment_id.to_string());
                self.state.snapshot.inference.status_line = format!(
                    "Completed · {} tokens · {}",
                    output.tokens.len(),
                    output.stop_reason.as_str()
                );
                self.state.set_status("Generation finished.");
            }
            Err(error) => {
                self.state.snapshot.inference.phase = Some(InferencePhase::Failed);
                self.state.snapshot.inference.error = Some(error.clone());
                self.state.snapshot.inference.status_line = "Generation failed".to_owned();
                self.state.set_status(error);
            }
        }
        self.refresh_replica_views();
        self.broadcast_replica_status();
    }

    fn on_replica_status(&mut self, from_peer: NodeId, status: ReplicaStatusMessage) {
        if let Some(peer) = self.state.peers.get_mut(&from_peer) {
            peer.replica_model_line = Some(status.model_line);
            peer.replica_backend = Some(status.backend);
            peer.replica_ready = status.ready;
            peer.replica_healthy = status.healthy;
            peer.replica_active_requests = status.active_requests;
            peer.replica_max_concurrent_requests = status.max_concurrent_requests.max(1);
            peer.replica_deployment_id = Some(status.deployment_id.to_string());
            self.state.rebuild_peer_summaries();
        }
        self.refresh_replica_views();
    }

    async fn on_remote_inference_request(
        &mut self,
        from_peer: NodeId,
        request: InferenceRequestSpec,
    ) {
        if self.pipeline_stage.as_ref().is_some_and(|stage| {
            matches!(
                stage.assignment.role,
                StageRole::First | StageRole::Final | StageRole::Complete
            )
        }) {
            self.on_pipeline_inference_request(from_peer, request);
            return;
        }

        let reject = |this: &mut Self, reason: String| {
            if let Some(session) = this.sessions.get(&from_peer) {
                let event = TokenResultEvent {
                    deployment_id: request.deployment_id,
                    request_id: request.request_id,
                    token_id: 0,
                    token_index: 0,
                    is_last: true,
                    stop_reason: Some(StopReason::Error),
                    sequence_length: 0,
                };
                let _ = session
                    .commands
                    .try_send(SessionCommand::SendTokenResult { event });
            }
            warn!(%from_peer, %reason, "rejected remote inference request");
        };

        let Some(engine_ref) = self.inference_engine.as_ref() else {
            reject(self, "no local model loaded".to_owned());
            return;
        };
        if engine_ref.deployment_id != request.deployment_id {
            reject(self, "deployment mismatch".to_owned());
            return;
        }
        if self.local_active_requests >= engine_ref.max_concurrent_requests() {
            reject(self, "no free local slots".to_owned());
            return;
        }
        let Some(mut engine) = self.inference_engine.take() else {
            reject(self, "engine unavailable".to_owned());
            return;
        };
        let (cancel_tx, cancel_rx) = watch::channel(false);
        self.serving_remote_requests.insert(
            request.request_id,
            ServingRemoteRequest {
                owner_node_id: from_peer,
                cancel: cancel_tx,
            },
        );
        self.local_active_requests = self.local_active_requests.saturating_add(1);
        self.refresh_replica_views();
        self.broadcast_replica_status();
        let event_tx = self.event_tx.clone();
        let prompt = String::new();
        tokio::task::spawn_blocking(move || {
            let result = engine
                .generate_from_tokens(
                    &request.input_token_ids,
                    request.sampling,
                    &request.stop_token_ids,
                    request.request_id,
                    |_| {},
                    || !*cancel_rx.borrow(),
                )
                .map_err(|error| error.to_string());
            let _ = event_tx.blocking_send(RuntimeEvent::GenerationFinished {
                prompt,
                engine: Box::new(engine),
                result,
                request_id: request.request_id,
                remote_owner: Some(from_peer),
            });
        });
    }
    fn on_remote_token(&mut self, event: TokenResultEvent) {
        let Some(pending) = self.pending_remote_generation.as_mut() else {
            return;
        };
        if pending.request_id != event.request_id {
            return;
        }
        if event.token_id != 0 || !event.is_last {
            pending.token_ids.push(event.token_id);
            self.state.snapshot.inference.generated_tokens = pending.token_ids.len() as u32;
            self.state.snapshot.inference.last_token_id = Some(event.token_id);
        }
        if let Some(tokenizer) = self.coordinator_tokenizer.as_ref() {
            match tokenizer.decode_stream(&pending.token_ids) {
                Ok(text) => self.state.snapshot.inference.output_text = text,
                Err(error) => self.state.snapshot.inference.error = Some(error.to_string()),
            }
        }
        if event.is_last {
            let peer_node_id = pending.peer_node_id;
            let prompt = pending.prompt.clone();
            self.pending_remote_generation = None;
            if let Some(peer) = self.state.peers.get_mut(&peer_node_id) {
                peer.replica_active_requests = peer.replica_active_requests.saturating_sub(1);
                peer.replica_ready = peer.replica_healthy
                    && peer.replica_active_requests < peer.replica_max_concurrent_requests;
                self.state.rebuild_peer_summaries();
            }
            self.state.snapshot.inference.busy = false;
            self.state.snapshot.inference.prompt = prompt;
            self.state.snapshot.inference.phase = Some(InferencePhase::Ready);
            self.state.snapshot.inference.stop_reason = event
                .stop_reason
                .map(|reason| reason.as_str().to_owned())
                .or_else(|| Some(StopReason::Eos.as_str().to_owned()));
            if event.stop_reason == Some(StopReason::Error) {
                self.state.snapshot.inference.phase = Some(InferencePhase::Failed);
                if self.state.snapshot.inference.error.is_none() {
                    self.state.snapshot.inference.error =
                        Some("Remote replica reported generation error.".to_owned());
                }
                self.state.snapshot.inference.status_line = "Remote generation failed".to_owned();
            } else {
                self.state.snapshot.inference.status_line = format!(
                    "Completed on remote · {} tokens · {}",
                    self.state.snapshot.inference.generated_tokens,
                    self.state
                        .snapshot
                        .inference
                        .stop_reason
                        .clone()
                        .unwrap_or_else(|| "eos".to_owned())
                );
                self.state.set_status("Remote generation finished.");
            }
            self.refresh_replica_views();
        }
    }
    fn spawn_pipeline_stage_load(
        &mut self,
        deployment_id: String,
        model_line: String,
        num_layers: u32,
        stage_index: u16,
        role: StageRole,
        layer_start: u32,
        layer_end: u32,
        node_ids: Vec<String>,
    ) {
        let Some(resolved) = self.resolved_model.clone() else {
            self.state.snapshot.inference.error =
                Some("Probe/resolve the model before pipeline load.".to_owned());
            return;
        };
        if self.state.snapshot.inference.busy || self.state.snapshot.models.busy {
            return;
        }
        let Ok(deployment_id) = DeploymentId::parse_hex(&deployment_id) else {
            self.state.snapshot.inference.error = Some("Invalid deployment id.".to_owned());
            return;
        };
        let mut parsed_nodes = Vec::with_capacity(node_ids.len());
        for node_id in &node_ids {
            match NodeId::parse_hex(node_id) {
                Ok(id) => parsed_nodes.push(id),
                Err(error) => {
                    self.state.snapshot.inference.error =
                        Some(format!("Invalid pipeline node id: {error}"));
                    return;
                }
            }
        }
        if let Err(error) = LayerRange::new(layer_start, layer_end) {
            self.state.snapshot.inference.error = Some(error);
            return;
        }
        let placement = match PlacementPlan::split_even(
            deployment_id,
            model_line.clone(),
            num_layers,
            &parsed_nodes,
        ) {
            Ok(plan) => plan,
            Err(error) => {
                self.state.snapshot.inference.error = Some(error);
                return;
            }
        };
        let Some(assignment) = placement
            .stages
            .iter()
            .find(|stage| stage.stage_index == stage_index)
            .cloned()
        else {
            self.state.snapshot.inference.error =
                Some(format!("stage_index {stage_index} missing from placement"));
            return;
        };
        if assignment.role != role
            || assignment.layer_range.start != layer_start
            || assignment.layer_range.end != layer_end
        {
            self.state.snapshot.inference.error = Some(
                "LoadPipelineStage assignment does not match split_even placement".to_owned(),
            );
            return;
        }
        let local_id = self.state.identity.as_ref().map(|identity| identity.node_id);
        if local_id != Some(assignment.node_id) {
            self.state.snapshot.inference.error =
                Some("LoadPipelineStage node_id is not this local node".to_owned());
            return;
        }

        let plan = match build_stage_plan(
            deployment_id.to_string(),
            &resolved,
            assignment.role,
            assignment.layer_range,
        ) {
            Ok(plan) => plan,
            Err(error) => {
                self.state.snapshot.inference.error = Some(error.to_string());
                return;
            }
        };
        let provider = match self.build_provider() {
            Ok(provider) => provider,
            Err(error) => {
                self.state.snapshot.inference.error = Some(error);
                return;
            }
        };
        let existing = self.store.list_model_cache_entries().unwrap_or_default();
        let cache_root = self.paths.model_cache_dir.clone();
        let event_tx = self.event_tx.clone();
        self.state.snapshot.inference.busy = true;
        self.state.snapshot.models.busy = true;
        self.state.snapshot.inference.phase = Some(InferencePhase::Loading);
        self.state.snapshot.inference.error = None;
        self.state.snapshot.models.error = None;
        self.state.snapshot.models.progress = None;
        self.state.snapshot.inference.status_line = format!(
            "Preparing pipeline stage {} ({}, layers {}..{}, {} bytes)…",
            assignment.stage_index,
            assignment.role.as_str(),
            assignment.layer_range.start,
            assignment.layer_range.end,
            plan.disk_bytes_required
        );
        self.state.snapshot.models.status_line = self.state.snapshot.inference.status_line.clone();
        self.state.set_status("Preparing assigned stage tensors…");

        struct ChannelProgress {
            tx: mpsc::Sender<RuntimeEvent>,
        }
        impl ProgressSink for ChannelProgress {
            fn on_progress(&mut self, event: DownloadProgressEvent) {
                let _ = self.tx.try_send(RuntimeEvent::ModelProgress(event.progress));
            }
        }

        tokio::spawn(async move {
            let mut progress = ChannelProgress {
                tx: event_tx.clone(),
            };
            let prepared = match prepare_plan(
                &provider,
                &resolved,
                &plan,
                &cache_root,
                &existing,
                &mut progress,
            )
            .await
            {
                Ok(prepared) => prepared,
                Err(error) => {
                    let _ = event_tx
                        .send(RuntimeEvent::PipelineStageLoaded(Err(error.to_string())))
                        .await;
                    return;
                }
            };

            let load_tx = event_tx.clone();
            let prepared_for_load = prepared.clone();
            let load_result = tokio::task::spawn_blocking(move || {
                StageWorker::load_from_prepared(
                    assignment.stage_index,
                    assignment.role,
                    assignment.layer_range,
                    &resolved,
                    &prepared_for_load,
                    &cache_root,
                    true,
                    None,
                )
                .map(|worker| {
                    (
                        Box::new(LocalPipelineStage {
                            placement,
                            assignment,
                            worker,
                            model_line,
                        }),
                        prepared_for_load,
                    )
                })
                .map_err(|error| error.to_string())
            })
            .await;
            let result = match load_result {
                Ok(result) => result,
                Err(error) => Err(format!("pipeline stage load join failed: {error}")),
            };
            let _ = load_tx
                .send(RuntimeEvent::PipelineStageLoaded(result))
                .await;
        });
    }

    fn on_pipeline_stage_loaded(
        &mut self,
        result: Result<(Box<LocalPipelineStage>, PrepareResult), String>,
    ) {
        self.state.snapshot.inference.busy = false;
        self.state.snapshot.models.busy = false;
        self.state.snapshot.models.progress = None;
        match result {
            Ok((stage, prepared)) => {
                for entry in &prepared.cache_entries {
                    if let Err(error) = self.store.upsert_model_cache_entry(entry) {
                        warn!(%error, "failed to persist stage cache entry");
                    }
                }
                self.refresh_model_cache_view();
                self.state.snapshot.models.resolved_identity = Some(prepared.identity.clone());
                self.state.snapshot.models.last_prepare_summary = Some(prepared.summary.clone());
                self.state.snapshot.models.error = None;
                self.state.snapshot.models.status_line = prepared.summary.clone();
                self.last_prepare = Some(prepared);

                self.inference_engine = None;
                self.pipeline_requests.clear();
                self.ensure_coordinator_tokenizer();
                self.state.snapshot.inference.phase = Some(InferencePhase::Ready);
                self.state.snapshot.inference.error = None;
                self.state.snapshot.inference.deployment_id =
                    Some(stage.placement.deployment_id.to_string());
                self.state.snapshot.inference.model_line = Some(stage.model_line.clone());
                self.state.snapshot.inference.backend =
                    Some(stage.worker.backend().as_str().to_owned());
                self.state.snapshot.inference.status_line = format!(
                    "Pipeline stage {} ready ({}, layers {}..{}) on {}",
                    stage.assignment.stage_index,
                    stage.assignment.role.as_str(),
                    stage.assignment.layer_range.start,
                    stage.assignment.layer_range.end,
                    stage.worker.backend().as_str()
                );
                self.state.set_status(format!(
                    "Pipeline stage {} ready.",
                    stage.assignment.stage_index
                ));
                self.pipeline_stage = Some(*stage);
                self.local_active_requests = 0;
                self.refresh_replica_views();
                self.broadcast_replica_status();
            }
            Err(error) => {
                self.state.snapshot.inference.phase = Some(InferencePhase::Failed);
                self.state.snapshot.inference.error = Some(error.clone());
                self.state.snapshot.inference.status_line = "Pipeline stage load failed".to_owned();
                self.state.snapshot.models.error = Some(error.clone());
                self.state.snapshot.models.status_line = "Stage prepare/load failed".to_owned();
                self.state.set_status(error);
            }
        }
    }

    fn try_spawn_pipeline_generation(
        &mut self,
        prompt: String,
        params: SamplingParams,
    ) -> Option<()> {
        let stage = self.pipeline_stage.as_ref()?;
        let placement = stage.placement.clone();
        let first = placement.stages.first()?.clone();
        let final_stage = placement.stages.last()?.clone();
        let deployment_id = placement.deployment_id;
        let model_line = stage.model_line.clone();
        let backend = stage.worker.backend().as_str().to_owned();
        let local_id = self.state.identity.as_ref().map(|identity| identity.node_id)?;

        let tokenizer = self.coordinator_tokenizer.as_ref()?;
        let token_ids = match tokenizer.encode_chat(None, &prompt) {
            Ok(ids) => ids,
            Err(error) => {
                self.state.snapshot.inference.error = Some(error.to_string());
                return Some(());
            }
        };
        let prompt_len = token_ids.len() as u32;
        let request_id = RequestId::new();
        let request = InferenceRequestSpec {
            deployment_id,
            request_id,
            input_token_ids: token_ids,
            sampling: params,
            stop_token_ids: Vec::new(),
            return_logprobs: false,
        };

        // Ensure final stage has sampling state before first activation arrives.
        if final_stage.node_id == local_id {
            self.seed_final_pipeline_request(&request, local_id, prompt_len);
        } else if let Some(session) = self.sessions.get(&final_stage.node_id) {
            let _ = session.commands.try_send(SessionCommand::SendInferenceRequest {
                request: request.clone(),
            });
        }

        if first.node_id == local_id {
            self.on_pipeline_inference_request(local_id, request);
        } else {
            let session = self.sessions.get(&first.node_id)?;
            if session
                .commands
                .try_send(SessionCommand::SendInferenceRequest {
                    request: request.clone(),
                })
                .is_err()
            {
                self.state.snapshot.inference.error =
                    Some("Failed to send pipeline inference request.".to_owned());
                return Some(());
            }
        }

        self.pending_remote_generation = Some(PendingRemoteGeneration {
            peer_node_id: first.node_id,
            request_id,
            deployment_id,
            prompt: prompt.clone(),
            token_ids: Vec::new(),
            pipeline: true,
        });
        self.state.snapshot.inference.busy = true;
        self.state.snapshot.inference.phase = Some(InferencePhase::Generating);
        self.state.snapshot.inference.prompt = prompt;
        self.state.snapshot.inference.output_text.clear();
        self.state.snapshot.inference.error = None;
        self.state.snapshot.inference.stop_reason = None;
        self.state.snapshot.inference.generated_tokens = 0;
        self.state.snapshot.inference.last_token_id = None;
        self.state.snapshot.inference.routed_node_id = Some(first.node_id.to_string());
        self.state.snapshot.inference.model_line = Some(model_line);
        self.state.snapshot.inference.backend = Some(backend);
        self.state.snapshot.inference.deployment_id = Some(deployment_id.to_string());
        self.state.snapshot.inference.status_line = "Generating on pipeline…".to_owned();
        self.state.set_status("Pipeline generation started.");
        Some(())
    }

    fn seed_final_pipeline_request(
        &mut self,
        request: &InferenceRequestSpec,
        owner_node_id: NodeId,
        prompt_len: u32,
    ) {
        let Some(stage) = self.pipeline_stage.as_ref() else {
            return;
        };
        if !stage.assignment.role.emits_logits() {
            return;
        }
        if stage.placement.deployment_id != request.deployment_id {
            return;
        }
        let first_stage_node = stage
            .placement
            .stages
            .first()
            .map(|item| item.node_id)
            .unwrap_or(stage.assignment.node_id);
        self.pipeline_requests
            .entry(request.request_id)
            .and_modify(|state| {
                state.sampling = request.sampling;
                state.prompt_len = prompt_len;
                state.stop_token_ids = request.stop_token_ids.clone();
            })
            .or_insert(PipelineRequestState {
                request_id: request.request_id,
                deployment_id: request.deployment_id,
                owner_node_id,
                first_stage_node,
                next_stage_node: None,
                sampler: None,
                sampling: request.sampling,
                prompt_len,
                stop_token_ids: request.stop_token_ids.clone(),
            });
    }

    fn on_pipeline_inference_request(&mut self, from_peer: NodeId, request: InferenceRequestSpec) {
        let Some(stage_role) = self.pipeline_stage.as_ref().map(|stage| stage.assignment.role) else {
            self.reject_pipeline_request(from_peer, &request, "no local pipeline stage".to_owned());
            return;
        };

        if stage_role.emits_logits() && stage_role != StageRole::First && stage_role != StageRole::Complete
        {
            let prompt_len = request.input_token_ids.len() as u32;
            self.seed_final_pipeline_request(&request, from_peer, prompt_len);
            return;
        }

        if stage_role != StageRole::First && stage_role != StageRole::Complete {
            self.reject_pipeline_request(from_peer, &request, "local stage rejects token ids".to_owned());
            return;
        }

        let reject_reason = {
            let stage = self.pipeline_stage.as_ref().unwrap();
            if stage.placement.deployment_id != request.deployment_id {
                Some("deployment mismatch".to_owned())
            } else if !self.pipeline_requests.is_empty() && stage_role == StageRole::First {
                // Final may already be seeded under same request id when local is complete-only.
                if !self.pipeline_requests.contains_key(&request.request_id) {
                    Some("pipeline stage busy".to_owned())
                } else {
                    None
                }
            } else if self.local_active_requests > 0 && !self.pipeline_requests.contains_key(&request.request_id)
            {
                Some("pipeline stage busy".to_owned())
            } else {
                None
            }
        };
        if let Some(reason) = reject_reason {
            self.reject_pipeline_request(from_peer, &request, reason);
            return;
        }

        let (next_stage_node, first_stage_node, deployment_ok) = {
            let stage = self.pipeline_stage.as_ref().unwrap();
            (
                stage
                    .placement
                    .stages
                    .get(stage.assignment.stage_index as usize + 1)
                    .map(|item| item.node_id),
                stage.assignment.node_id,
                stage.placement.deployment_id == request.deployment_id,
            )
        };
        if !deployment_ok {
            self.reject_pipeline_request(from_peer, &request, "deployment mismatch".to_owned());
            return;
        }

        // Forward sampling state to final before activations arrive.
        if let Some(next) = next_stage_node {
            if let Some(session) = self.sessions.get(&next) {
                let _ = session
                    .commands
                    .try_send(SessionCommand::SendInferenceRequest {
                        request: request.clone(),
                    });
            }
        }

        let hop = {
            let stage = self.pipeline_stage.as_mut().unwrap();
            match stage.worker.prefill_from_tokens(
                request.deployment_id,
                request.request_id,
                &request.input_token_ids,
            ) {
                Ok(hop) => hop,
                Err(error) => {
                    self.reject_pipeline_request(from_peer, &request, error.to_string());
                    return;
                }
            }
        };

        self.pipeline_requests
            .entry(request.request_id)
            .and_modify(|state| {
                state.owner_node_id = from_peer;
                state.next_stage_node = next_stage_node;
                state.sampling = request.sampling;
                state.prompt_len = request.input_token_ids.len() as u32;
                state.stop_token_ids = request.stop_token_ids.clone();
            })
            .or_insert(PipelineRequestState {
                request_id: request.request_id,
                deployment_id: request.deployment_id,
                owner_node_id: from_peer,
                first_stage_node,
                next_stage_node,
                sampler: None,
                sampling: request.sampling,
                prompt_len: request.input_token_ids.len() as u32,
                stop_token_ids: request.stop_token_ids.clone(),
            });
        self.local_active_requests = self.local_active_requests.max(1);

        if let Err(error) = self.dispatch_stage_hop(request.request_id, hop) {
            self.fail_pipeline_request(request.request_id, error);
        }
    }

    fn reject_pipeline_request(
        &mut self,
        from_peer: NodeId,
        request: &InferenceRequestSpec,
        reason: String,
    ) {
        if let Some(session) = self.sessions.get(&from_peer) {
            let event = TokenResultEvent {
                deployment_id: request.deployment_id,
                request_id: request.request_id,
                token_id: 0,
                token_index: 0,
                is_last: true,
                stop_reason: Some(StopReason::Error),
                sequence_length: 0,
            };
            let _ = session
                .commands
                .try_send(SessionCommand::SendTokenResult { event });
        } else if self
            .pending_remote_generation
            .as_ref()
            .is_some_and(|pending| pending.request_id == request.request_id)
        {
            self.state.snapshot.inference.busy = false;
            self.state.snapshot.inference.phase = Some(InferencePhase::Failed);
            self.state.snapshot.inference.error = Some(reason.clone());
            self.pending_remote_generation = None;
        }
        warn!(%from_peer, %reason, "rejected pipeline inference request");
    }

    fn on_pipeline_next_token_feedback(&mut self, from_peer: NodeId, feedback: NextTokenFeedback) {
        let _ = from_peer;
        if feedback.is_last {
            if let Some(stage) = self.pipeline_stage.as_mut() {
                stage.worker.finish_request(feedback.request_id);
            }
            if self.pipeline_requests.remove(&feedback.request_id).is_some() {
                self.local_active_requests = self.local_active_requests.saturating_sub(1);
            }
            return;
        }

        let Some(stage) = self.pipeline_stage.as_mut() else {
            return;
        };
        if stage.assignment.role != StageRole::First {
            return;
        }
        if stage.placement.deployment_id != feedback.deployment_id {
            return;
        }
        if !self.pipeline_requests.contains_key(&feedback.request_id) {
            return;
        }

        let hop = match stage.worker.decode_from_token(
            feedback.deployment_id,
            feedback.request_id,
            feedback.token_id,
        ) {
            Ok(hop) => hop,
            Err(error) => {
                self.fail_pipeline_request(feedback.request_id, error.to_string());
                return;
            }
        };
        if let Err(error) = self.dispatch_stage_hop(feedback.request_id, hop) {
            self.fail_pipeline_request(feedback.request_id, error);
        }
    }

    fn on_pipeline_activation(&mut self, from_peer: NodeId, frame: ActivationFrame) {
        let Some(stage) = self.pipeline_stage.as_ref() else {
            warn!("dropped activation; no local pipeline stage");
            return;
        };
        if frame.header.deployment_id != stage.placement.deployment_id {
            warn!("dropped activation; deployment mismatch");
            return;
        }
        if frame.header.destination_stage != stage.assignment.stage_index {
            warn!(
                dest = frame.header.destination_stage,
                local = stage.assignment.stage_index,
                "dropped activation; destination mismatch"
            );
            return;
        }

        let request_id = frame.header.request_id;
        let deployment_id = frame.header.deployment_id;
        let transfer_kind = frame.header.transfer_kind;

        if transfer_kind == mesh_core::TransferKind::Prefill
            && !self.pipeline_requests.contains_key(&request_id)
        {
            let next_stage_node = stage
                .placement
                .stages
                .get(stage.assignment.stage_index as usize + 1)
                .map(|item| item.node_id);
            let first_stage_node = stage
                .placement
                .stages
                .first()
                .map(|item| item.node_id)
                .unwrap_or(stage.assignment.node_id);
            self.pipeline_requests.insert(
                request_id,
                PipelineRequestState {
                    request_id,
                    deployment_id,
                    owner_node_id: from_peer,
                    first_stage_node,
                    next_stage_node,
                    sampler: None,
                    sampling: SamplingParams {
                        temperature: 0.0,
                        top_k: 0,
                        top_p: 1.0,
                        repetition_penalty: 1.0,
                        seed: 0,
                        max_new_tokens: 1,
                    },
                    prompt_len: frame.header.used_dimensions().get(1).copied().unwrap_or(1) as u32,
                    stop_token_ids: Vec::new(),
                },
            );
            self.local_active_requests = self.local_active_requests.saturating_add(1);
        }

        let incoming = StageActivation {
            header: frame.header,
            payload: frame.payload,
        };
        let hop = {
            let stage = self.pipeline_stage.as_mut().unwrap();
            match stage
                .worker
                .forward_activation(deployment_id, request_id, incoming)
            {
                Ok(hop) => hop,
                Err(error) => {
                    self.fail_pipeline_request(request_id, error.to_string());
                    return;
                }
            }
        };

        if let Err(error) = self.dispatch_stage_hop(request_id, hop) {
            self.fail_pipeline_request(request_id, error);
        }
    }

    fn dispatch_stage_hop(
        &mut self,
        request_id: RequestId,
        hop: StageHop,
    ) -> Result<(), String> {
        match hop {
            StageHop::Activation(activation) => {
                let next = self
                    .pipeline_requests
                    .get(&request_id)
                    .and_then(|state| state.next_stage_node)
                    .ok_or_else(|| "missing next stage peer".to_owned())?;
                let session = self
                    .sessions
                    .get(&next)
                    .ok_or_else(|| "next stage peer not connected".to_owned())?;
                session
                    .commands
                    .try_send(SessionCommand::SendActivation {
                        header: activation.header,
                        payload: activation.payload,
                    })
                    .map_err(|_| "failed to queue activation send".to_owned())?;
                Ok(())
            }
            StageHop::Logits(logits) => self.handle_final_logits(request_id, logits),
        }
    }

    fn handle_final_logits(
        &mut self,
        request_id: RequestId,
        logits: Vec<f32>,
    ) -> Result<(), String> {
        let stage = self
            .pipeline_stage
            .as_ref()
            .ok_or_else(|| "missing pipeline stage".to_owned())?;
        if !stage.assignment.role.emits_logits() {
            return Err("non-final stage produced logits".to_owned());
        }
        let vocab_size = stage.worker.vocab_size();
        let context_limit = stage.worker.context_limit();
        let local_id = self
            .state
            .identity
            .as_ref()
            .map(|identity| identity.node_id);

        let (event, feedback, owner, first_stage, is_last) = {
            let state = self
                .pipeline_requests
                .get_mut(&request_id)
                .ok_or_else(|| "missing pipeline request state".to_owned())?;
            if state.sampler.is_none() {
                let prompt_stub = vec![0u32; state.prompt_len.max(1) as usize];
                state.sampler = Some(Sampler::new(
                    state.sampling,
                    vocab_size,
                    151_645,
                    state.stop_token_ids.clone(),
                    context_limit,
                    &prompt_stub,
                )?);
            }
            let sampler = state
                .sampler
                .as_mut()
                .ok_or_else(|| "final stage missing sampler".to_owned())?;
            let outcome = sampler.sample(&logits)?;
            let event = TokenResultEvent {
                deployment_id: state.deployment_id,
                request_id,
                token_id: outcome.token_id,
                token_index: outcome.token_index,
                is_last: outcome.is_last,
                stop_reason: outcome.stop_reason,
                sequence_length: outcome.sequence_length,
            };
            let feedback = NextTokenFeedback {
                deployment_id: state.deployment_id,
                request_id,
                token_id: outcome.token_id,
                token_index: outcome.token_index,
                is_last: outcome.is_last,
            };
            (
                event,
                feedback,
                state.owner_node_id,
                state.first_stage_node,
                outcome.is_last,
            )
        };

        if let Some(pending) = self.pending_remote_generation.as_ref() {
            if pending.pipeline && pending.request_id == request_id {
                self.on_remote_token(event.clone());
            }
        }
        if Some(owner) != local_id {
            if let Some(session) = self.sessions.get(&owner) {
                let _ = session
                    .commands
                    .try_send(SessionCommand::SendTokenResult {
                        event: event.clone(),
                    });
            }
        } else if !self
            .pending_remote_generation
            .as_ref()
            .is_some_and(|pending| pending.pipeline && pending.request_id == request_id)
        {
            // Local coordinator without pending marker still surfaces tokens.
            self.on_remote_token(event.clone());
        }

        if Some(first_stage) != local_id {
            if let Some(session) = self.sessions.get(&first_stage) {
                let _ = session
                    .commands
                    .try_send(SessionCommand::SendNextTokenFeedback {
                        feedback: feedback.clone(),
                    });
            }
        } else if !is_last {
            self.on_pipeline_next_token_feedback(first_stage, feedback);
        }

        if is_last {
            if let Some(stage) = self.pipeline_stage.as_mut() {
                stage.worker.finish_request(request_id);
            }
            if self.pipeline_requests.remove(&request_id).is_some() {
                self.local_active_requests = self.local_active_requests.saturating_sub(1);
            }
        }

        Ok(())
    }

    fn fail_pipeline_request(&mut self, request_id: RequestId, reason: String) {
        warn!(%request_id, %reason, "pipeline request failed");
        let state = self.pipeline_requests.remove(&request_id);
        if let Some(stage) = self.pipeline_stage.as_mut() {
            stage.worker.cancel(request_id);
        }
        self.local_active_requests = self.local_active_requests.saturating_sub(1);
        if let Some(state) = state {
            let event = TokenResultEvent {
                deployment_id: state.deployment_id,
                request_id,
                token_id: 0,
                token_index: 0,
                is_last: true,
                stop_reason: Some(StopReason::Error),
                sequence_length: 0,
            };
            if let Some(pending) = self.pending_remote_generation.as_ref() {
                if pending.pipeline && pending.request_id == request_id {
                    self.on_remote_token(event.clone());
                }
            }
            if let Some(session) = self.sessions.get(&state.owner_node_id) {
                let _ = session
                    .commands
                    .try_send(SessionCommand::SendTokenResult { event });
            }
            if state.first_stage_node
                != self
                    .state
                    .identity
                    .as_ref()
                    .map(|id| id.node_id)
                    .unwrap_or(state.first_stage_node)
            {
                if let Some(session) = self.sessions.get(&state.first_stage_node) {
                    let _ = session
                        .commands
                        .try_send(SessionCommand::SendNextTokenFeedback {
                            feedback: NextTokenFeedback {
                                deployment_id: state.deployment_id,
                                request_id,
                                token_id: 0,
                                token_index: 0,
                                is_last: true,
                            },
                        });
                }
            }
        }
    }

    fn cancel_pipeline_request(&mut self, deployment_id: DeploymentId, request_id: RequestId) {
        let Some(stage) = self.pipeline_stage.as_mut() else {
            return;
        };
        if stage.placement.deployment_id != deployment_id
            && self
                .pipeline_requests
                .get(&request_id)
                .map(|state| state.deployment_id)
                != Some(deployment_id)
        {
            return;
        }
        stage.worker.cancel(request_id);
        if self.pipeline_requests.remove(&request_id).is_some() {
            self.local_active_requests = self.local_active_requests.saturating_sub(1);
        }
    }

    fn cancel_pipeline_for_peer(&mut self, peer_node_id: NodeId) {
        let ids: Vec<_> = self
            .pipeline_requests
            .iter()
            .filter_map(|(id, state)| {
                if state.owner_node_id == peer_node_id
                    || state.first_stage_node == peer_node_id
                    || state.next_stage_node == Some(peer_node_id)
                {
                    Some(*id)
                } else {
                    None
                }
            })
            .collect();
        for id in ids {
            if let Some(stage) = self.pipeline_stage.as_mut() {
                stage.worker.cancel(id);
            }
            self.pipeline_requests.remove(&id);
            self.local_active_requests = self.local_active_requests.saturating_sub(1);
        }
    }

    fn local_replica_view(&self) -> Option<ReplicaEndpointView> {
        let identity = self.state.identity.as_ref()?;
        if let Some(engine) = self.inference_engine.as_ref() {
            let max = engine.max_concurrent_requests();
            let health = if self.local_active_requests >= max {
                ReplicaHealth::Busy
            } else {
                ReplicaHealth::Ready
            };
            return Some(ReplicaEndpointView {
                node_id: identity.node_id.to_string(),
                display_name: identity.display_name.clone(),
                deployment_id: engine.deployment_id.to_string(),
                model_line: engine.model_line.clone(),
                backend: engine.backend.as_str().to_owned(),
                ready: health == ReplicaHealth::Ready,
                healthy: true,
                active_requests: self.local_active_requests,
                max_concurrent_requests: max,
                health,
                local: true,
            });
        }
        let stage = self.pipeline_stage.as_ref()?;
        let max = 1u32;
        let health = if self.local_active_requests >= max {
            ReplicaHealth::Busy
        } else {
            ReplicaHealth::Ready
        };
        Some(ReplicaEndpointView {
            node_id: identity.node_id.to_string(),
            display_name: identity.display_name.clone(),
            deployment_id: stage.placement.deployment_id.to_string(),
            model_line: stage.model_line.clone(),
            backend: stage.worker.backend().as_str().to_owned(),
            ready: health == ReplicaHealth::Ready,
            healthy: true,
            active_requests: self.local_active_requests,
            max_concurrent_requests: max,
            health,
            local: true,
        })
    }

    fn refresh_replica_views(&mut self) {
        let mut replicas = Vec::new();
        if let Some(local) = self.local_replica_view() {
            replicas.push(local);
        }
        for peer in self.state.peers.values() {
            if !peer.connected {
                continue;
            }
            let Some(model_line) = peer.replica_model_line.clone() else {
                continue;
            };
            let Some(deployment_id) = peer.replica_deployment_id.clone() else {
                continue;
            };
            let max = peer.replica_max_concurrent_requests.max(1);
            let health = if !peer.replica_healthy {
                ReplicaHealth::Unhealthy
            } else if peer.replica_active_requests >= max || !peer.replica_ready {
                ReplicaHealth::Busy
            } else {
                ReplicaHealth::Ready
            };
            replicas.push(ReplicaEndpointView {
                node_id: peer.node_id.to_string(),
                display_name: peer.display_name.clone(),
                deployment_id,
                model_line,
                backend: peer
                    .replica_backend
                    .clone()
                    .unwrap_or_else(|| "unknown".to_owned()),
                ready: health == ReplicaHealth::Ready,
                healthy: peer.replica_healthy,
                active_requests: peer.replica_active_requests,
                max_concurrent_requests: max,
                health,
                local: false,
            });
        }
        replicas.sort_by(|left, right| {
            left.display_name
                .cmp(&right.display_name)
                .then_with(|| left.node_id.cmp(&right.node_id))
        });
        self.state.snapshot.inference.replicas = replicas;
    }

    fn local_replica_status_message(&self) -> Option<ReplicaStatusMessage> {
        let view = self.local_replica_view()?;
        ReplicaStatusMessage::from_local_view(&view).ok()
    }

    fn broadcast_replica_status(&self) {
        let Some(status) = self.local_replica_status_message() else {
            return;
        };
        for session in self.sessions.values() {
            let _ = session
                .commands
                .try_send(SessionCommand::SendReplicaStatus {
                    status: status.clone(),
                });
        }
    }

    fn announce_replica_status_to(&self, peer_node_id: NodeId) {
        let Some(status) = self.local_replica_status_message() else {
            return;
        };
        if let Some(session) = self.sessions.get(&peer_node_id) {
            let _ = session
                .commands
                .try_send(SessionCommand::SendReplicaStatus { status });
        }
    }

    fn fail_pending_remote_for_peer(&mut self, peer_node_id: NodeId, message: String) {
        let Some(pending) = self.pending_remote_generation.as_ref() else {
            return;
        };
        if pending.peer_node_id != peer_node_id {
            return;
        }
        self.pending_remote_generation = None;
        self.state.snapshot.inference.busy = false;
        self.state.snapshot.inference.phase = Some(InferencePhase::Failed);
        self.state.snapshot.inference.error = Some(message);
        self.state.snapshot.inference.status_line = "Remote replica disconnected".to_owned();
        self.refresh_replica_views();
    }

    fn cancel_serving_for_peer(&mut self, peer_node_id: NodeId) {
        let ids: Vec<_> = self
            .serving_remote_requests
            .iter()
            .filter_map(|(id, serving)| {
                if serving.owner_node_id == peer_node_id {
                    Some(*id)
                } else {
                    None
                }
            })
            .collect();
        for id in ids {
            if let Some(serving) = self.serving_remote_requests.remove(&id) {
                let _ = serving.cancel.send(true);
            }
        }
    }

    fn publish(&self) {
        let _ = self.snapshot_tx.send(self.state.snapshot.clone());
    }

    fn publish_resources(&mut self) {
        self.state.snapshot.resources = self.resources.view();
    }

    fn sweep_reservations(&mut self) {
        let expired = self.resources.expire_due(now_unix_ms());
        if expired.is_empty() {
            return;
        }
        for reservation in expired {
            if let Err(error) = self.store.delete_reservation(reservation.reservation_id) {
                warn!(%error, "failed to delete expired reservation");
            }
        }
        self.publish_resources();
        self.publish();
    }

    fn persist_reservation(&mut self, reservation: &mesh_core::LocalReservation) {
        if let Err(error) = self.store.upsert_reservation(reservation) {
            warn!(%error, "failed to persist reservation");
        }
        self.publish_resources();
    }

    fn delete_persisted_reservation(&mut self, reservation_id: ReservationId) {
        if let Err(error) = self.store.delete_reservation(reservation_id) {
            warn!(%error, "failed to delete reservation");
        }
        self.publish_resources();
    }

    fn release_owner_reservations(&mut self, owner_node_id: NodeId) {
        let released = self.resources.release_owner(owner_node_id);
        if released.is_empty() {
            return;
        }
        if let Err(error) = self.store.delete_reservations_for_owner(owner_node_id) {
            warn!(%error, "failed to clear owner reservations");
        }
        self.publish_resources();
    }

    fn release_all_local_reservations(&mut self) {
        let _ = self.resources.release_all();
        if let Err(error) = self.store.clear_reservations() {
            warn!(%error, "failed to clear reservations");
        }
        self.publish_resources();
    }

    fn probe_amount_from_capacity(&self) -> ResourceAmount {
        let available = self.resources.available_amount(now_unix_ms());
        let gpus = available
            .gpus
            .into_iter()
            .filter(|gpu| gpu.memory_bytes > 0)
            .map(|gpu| GpuResourceAmount {
                device_stable_id: gpu.device_stable_id,
                memory_bytes: gpu.memory_bytes,
            })
            .collect::<Vec<_>>();
        ResourceAmount {
            system_memory_bytes: available
                .system_memory_bytes
                .min(64 * 1024 * 1024)
                .max(if available.system_memory_bytes > 0 {
                    1
                } else {
                    0
                }),
            disk_bytes: available.disk_bytes.min(128 * 1024 * 1024).max(
                if available.disk_bytes > 0 { 1 } else { 0 },
            ),
            execution_slots: available.execution_slots.max(1),
            gpus,
        }
    }

    fn run_local_reservation_probe(&mut self) -> Result<(), String> {
        let owner = self
            .state
            .identity
            .as_ref()
            .map(|identity| identity.node_id)
            .unwrap_or_else(|| NodeId::from_bytes([0xEE; 32]));
        let amount = self.probe_amount_from_capacity();
        if amount.is_zero() {
            return Err("no local capacity available for a probe reservation".to_owned());
        }
        let request = ReserveRequest {
            deployment_id: DeploymentId::new(),
            reservation_id: ReservationId::new(),
            amount,
            lease_duration_ms: DEFAULT_HOLD_LEASE_MS,
            purpose: "local-probe".to_owned(),
        };
        match self.resources.reserve(owner, &request) {
            ReserveOutcome::Accepted(accepted) => {
                if let Some(reservation) = self
                    .resources
                    .active_reservations()
                    .into_iter()
                    .find(|item| item.reservation_id == accepted.reservation_id)
                {
                    self.persist_reservation(&reservation);
                } else {
                    self.publish_resources();
                }
                self.state.set_status(format!(
                    "Reserved local capacity until {}.",
                    accepted.expires_at_unix_ms
                ));
                Ok(())
            }
            ReserveOutcome::Rejected(rejected) => Err(rejected.reason),
        }
    }

    async fn handle_resource_query(
        &mut self,
        from_peer: NodeId,
        message_id: Bytes,
        query: ResourceQuery,
    ) {
        let offer = self.resources.offer(&query);
        self.publish_resources();
        self.publish();
        if let Some(session) = self.sessions.get(&from_peer) {
            let _ = session
                .commands
                .send(SessionCommand::SendResourceOffer {
                    offer,
                    in_reply_to: Some(message_id),
                })
                .await;
        }
    }

    async fn handle_reserve_request(
        &mut self,
        from_peer: NodeId,
        message_id: Bytes,
        request: ReserveRequest,
    ) {
        let outcome = self.resources.reserve(from_peer, &request);
        match outcome {
            ReserveOutcome::Accepted(accepted) => {
                if let Some(reservation) = self
                    .resources
                    .active_reservations()
                    .into_iter()
                    .find(|item| item.reservation_id == accepted.reservation_id)
                {
                    self.persist_reservation(&reservation);
                } else {
                    self.publish_resources();
                }
                self.publish();
                if let Some(session) = self.sessions.get(&from_peer) {
                    let _ = session
                        .commands
                        .send(SessionCommand::SendReserveAccepted {
                            accepted,
                            in_reply_to: Some(message_id),
                        })
                        .await;
                }
            }
            ReserveOutcome::Rejected(rejected) => {
                self.publish_resources();
                self.publish();
                if let Some(session) = self.sessions.get(&from_peer) {
                    let _ = session
                        .commands
                        .send(SessionCommand::SendReserveRejected {
                            rejected,
                            in_reply_to: Some(message_id),
                        })
                        .await;
                }
            }
        }
    }

    fn handle_reservation_commit(&mut self, from_peer: NodeId, commit: ReservationCommit) {
        match self.resources.commit(
            from_peer,
            commit.deployment_id,
            commit.reservation_id,
            commit.lease_duration_ms,
        ) {
            Ok(reservation) => {
                self.persist_reservation(&reservation);
                self.state.set_status(format!(
                    "Committed reservation {}.",
                    reservation.reservation_id.short_hex()
                ));
                self.publish();
            }
            Err(reason) => {
                warn!(%from_peer, %reason, "reservation commit rejected");
            }
        }
    }

    fn handle_reservation_release(&mut self, from_peer: NodeId, release: ReservationRelease) {
        match self.resources.release(
            Some(from_peer),
            Some(release.deployment_id),
            release.reservation_id,
        ) {
            Ok(reservation) => {
                self.delete_persisted_reservation(reservation.reservation_id);
                self.state.set_status(format!(
                    "Released reservation {} ({}).",
                    reservation.reservation_id.short_hex(),
                    if release.reason.is_empty() {
                        "done"
                    } else {
                        release.reason.as_str()
                    }
                ));
                self.publish();
            }
            Err(reason) => warn!(%from_peer, %reason, "reservation release ignored"),
        }
    }

    #[allow(dead_code)]
    async fn reserve_on_peer(
        &mut self,
        peer_node_id: NodeId,
        request: ReserveRequest,
    ) -> Result<mesh_core::ReserveAccepted, String> {
        let session = self
            .sessions
            .get(&peer_node_id)
            .ok_or_else(|| format!("peer {peer_node_id} is not connected"))?;
        let (tx, rx) = oneshot::channel();
        self.pending_remote_reserves
            .insert(request.reservation_id, tx);
        session
            .commands
            .send(SessionCommand::SendReserveRequest {
                request: request.clone(),
            })
            .await
            .map_err(|_| "failed to send reserve request".to_owned())?;
        match tokio::time::timeout(Duration::from_secs(10), rx).await {
            Ok(Ok(Ok(accepted))) => Ok(accepted),
            Ok(Ok(Err(rejected))) => Err(rejected.reason),
            Ok(Err(_)) => {
                self.pending_remote_reserves.remove(&request.reservation_id);
                Err("reserve response channel closed".to_owned())
            }
            Err(_) => {
                self.pending_remote_reserves.remove(&request.reservation_id);
                Err("reserve request timed out".to_owned())
            }
        }
    }
}

fn accept_enrollment(
    store: &mut Store,
    hello: EnrollmentHello,
    peer: PeerRecord,
) -> Result<(), (ErrorCode, String)> {
    let enrollment_id = hello
        .enrollment_id
        .ok_or((ErrorCode::InviteInvalid, "missing enrollment id".to_owned()))?;
    let secret = hello.enrollment_secret.ok_or((
        ErrorCode::InviteInvalid,
        "missing enrollment secret".to_owned(),
    ))?;

    match store.bind_invitation(enrollment_id, &secret, &peer, now_unix_ms()) {
        Ok(()) => Ok(()),
        Err(error) => {
            let message = error.to_string();
            let code = if message.contains("expired") {
                ErrorCode::InviteExpired
            } else if message.contains("another node") {
                ErrorCode::InviteAlreadyUsed
            } else if message.contains("not found") || message.contains("secret") {
                ErrorCode::InviteInvalid
            } else {
                ErrorCode::Internal
            };
            Err((code, message))
        }
    }
}

fn normalize_display_name(value: String) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "This PC".to_owned()
    } else {
        trimmed.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn two_nodes_enroll_over_localhost() {
        let inviter_dir = std::env::temp_dir().join(format!("mesh-inviter-{}", now_unix_ms()));
        let joiner_dir = std::env::temp_dir().join(format!("mesh-joiner-{}", now_unix_ms() + 1));
        let _ = std::fs::remove_dir_all(&inviter_dir);
        let _ = std::fs::remove_dir_all(&joiner_dir);

        let inviter = NodeRuntime::create("Inviter PC", StorePaths::isolated(&inviter_dir))
            .expect("inviter runtime");
        let joiner = NodeRuntime::create("Joiner PC", StorePaths::isolated(&joiner_dir))
            .expect("joiner runtime");
        let inviter_handle = inviter.handle();
        let joiner_handle = joiner.handle();

        let inviter_task = tokio::spawn(inviter.run());
        let joiner_task = tokio::spawn(joiner.run());

        wait_for(&inviter_handle, |snapshot| {
            snapshot.phase == RuntimePhase::AwaitingOnboarding
                && snapshot
                    .hardware
                    .as_ref()
                    .is_some_and(|hw| hw.cpu_logical_cores >= 1)
        })
        .await;
        inviter_handle
            .send(UiCommand::CreateMesh {
                display_name: "Inviter PC".to_owned(),
            })
            .await
            .expect("create mesh");
        wait_for(&inviter_handle, |snapshot| {
            snapshot.phase == RuntimePhase::Ready && snapshot.local.listen_addr.is_some()
        })
        .await;

        inviter_handle
            .send(UiCommand::CreateInvitation)
            .await
            .expect("create invitation");
        let invitation = wait_for(&inviter_handle, |snapshot| {
            snapshot.enrollment.invitation_text.is_some()
        })
        .await
        .enrollment
        .invitation_text
        .expect("invitation text");

        wait_for(&joiner_handle, |snapshot| {
            snapshot.phase == RuntimePhase::AwaitingOnboarding
        })
        .await;
        joiner_handle
            .send(UiCommand::SubmitInvitation { text: invitation })
            .await
            .expect("submit invitation");

        let joiner_snapshot = wait_for(&joiner_handle, |snapshot| {
            snapshot.phase == RuntimePhase::Ready && !snapshot.peers.is_empty()
        })
        .await;
        let inviter_snapshot =
            wait_for(&inviter_handle, |snapshot| !snapshot.peers.is_empty()).await;

        assert_eq!(joiner_snapshot.peers.len(), 1);
        assert!(joiner_snapshot.peers[0].connected);
        assert_eq!(inviter_snapshot.peers.len(), 1);
        assert!(inviter_snapshot.peers[0].connected);
        assert_eq!(
            joiner_snapshot.local.mesh_id,
            inviter_snapshot.local.mesh_id
        );
        assert!(
            joiner_snapshot
                .hardware
                .as_ref()
                .is_some_and(|hw| !hw.cpu_model.is_empty())
        );

        let measured = wait_for(&joiner_handle, |snapshot| {
            snapshot.peers.iter().any(|peer| {
                peer.link.as_ref().is_some_and(|link| {
                    link.delay.is_some()
                        && (link.to_peer_bandwidth.is_some() || link.from_peer_bandwidth.is_some())
                })
            })
        })
        .await;
        let link = measured.peers[0]
            .link
            .as_ref()
            .expect("link measurement present");
        assert!(link.delay.as_ref().expect("delay").rtt_ms > 0.0);
        assert!(
            link.to_peer_bandwidth
                .as_ref()
                .or(link.from_peer_bandwidth.as_ref())
                .is_some(),
            "expected at least one bandwidth direction"
        );

        inviter_handle.request_shutdown().ok();
        joiner_handle.request_shutdown().ok();
        let _ = inviter_task.await;
        let _ = joiner_task.await;
        let _ = std::fs::remove_dir_all(&inviter_dir);
        let _ = std::fs::remove_dir_all(&joiner_dir);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn two_coordinators_cannot_reserve_same_local_capacity() {
        let dir = std::env::temp_dir().join(format!("mesh-reserve-{}", now_unix_ms()));
        let _ = std::fs::remove_dir_all(&dir);

        let runtime =
            NodeRuntime::create("Reserve PC", StorePaths::isolated(&dir)).expect("runtime");
        let handle = runtime.handle();
        let task = tokio::spawn(runtime.run());

        wait_for(&handle, |snapshot| {
            snapshot.phase == RuntimePhase::AwaitingOnboarding
                && !snapshot.resources.capacity_line.is_empty()
        })
        .await;

        handle
            .send(UiCommand::CreateMesh {
                display_name: "Reserve PC".to_owned(),
            })
            .await
            .expect("create mesh");
        wait_for(&handle, |snapshot| snapshot.phase == RuntimePhase::Ready).await;

        handle
            .send(UiCommand::RunLocalReservationProbe)
            .await
            .expect("first probe");
        let first = wait_for(&handle, |snapshot| !snapshot.resources.active.is_empty()).await;
        assert_eq!(first.resources.active.len(), 1);
        assert!(first.status_message.contains("Reserved local capacity"));

        handle
            .send(UiCommand::RunLocalReservationProbe)
            .await
            .expect("second probe");
        tokio::time::sleep(Duration::from_millis(400)).await;
        let second = handle.snapshot();
        assert_eq!(
            second.resources.active.len(),
            1,
            "second coordinator-style probe must not double-book local capacity: {:?}",
            second.resources
        );
        assert!(
            second.status_message.contains("execution slots")
                || second.status_message.contains("exceed")
                || second.status_message.contains("memory")
                || !second.status_message.contains("Reserved local capacity"),
            "expected rejection status, got {}",
            second.status_message
        );

        handle
            .send(UiCommand::ReleaseAllLocalReservations)
            .await
            .expect("release");
        wait_for(&handle, |snapshot| snapshot.resources.active.is_empty()).await;

        handle.request_shutdown().ok();
        let _ = task.await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn model_selection_updates_snapshot() {
        let dir = std::env::temp_dir().join(format!("mesh-model-ui-{}", now_unix_ms()));
        let _ = std::fs::remove_dir_all(&dir);
        let runtime =
            NodeRuntime::create("Model PC", StorePaths::isolated(&dir)).expect("runtime");
        let handle = runtime.handle();
        let task = tokio::spawn(runtime.run());

        wait_for(&handle, |snapshot| {
            snapshot.phase == RuntimePhase::AwaitingOnboarding
        })
        .await;

        handle
            .send(UiCommand::SelectModel {
                reference: ModelReference::qwen3_4b(),
            })
            .await
            .expect("select model");
        let selected = wait_for(&handle, |snapshot| {
            snapshot
                .models
                .selected_reference
                .as_ref()
                .is_some_and(|item| item.repository == "Qwen/Qwen3-4B")
        })
        .await;
        assert!(selected.models.status_line.contains("Qwen3-4B"));
        assert!(!selected.models.cache.root.is_empty());

        handle.request_shutdown().ok();
        let _ = task.await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn p07_single_node_prepare_load_generate_smoke() {
        if std::env::var_os("MESH_P07_SMOKE").is_none() {
            eprintln!("skipping P07 host smoke; set MESH_P07_SMOKE=1 to run");
            return;
        }

        let dir = std::env::var_os("MESH_P07_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(std::env::temp_dir)
                    .join("mesh-p07-smoke")
            });
        std::fs::create_dir_all(&dir).expect("create smoke data dir");

        let max_new_tokens = std::env::var("MESH_P07_MAX_NEW_TOKENS")
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(8)
            .max(1);
        let prepare_timeout_secs = std::env::var("MESH_P07_PREPARE_TIMEOUT_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(6 * 60 * 60);
        let load_timeout_secs = std::env::var("MESH_P07_LOAD_TIMEOUT_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(60 * 60);
        let generate_timeout_secs = std::env::var("MESH_P07_GENERATE_TIMEOUT_SECS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(2 * 60 * 60);

        let runtime =
            NodeRuntime::create("P07 Smoke PC", StorePaths::isolated(&dir)).expect("runtime");
        let handle = runtime.handle();
        let task = tokio::spawn(runtime.run());

        let boot = wait_for_timeout(
            &handle,
            Duration::from_secs(60),
            |snapshot| {
                matches!(
                    snapshot.phase,
                    RuntimePhase::AwaitingOnboarding | RuntimePhase::Ready
                )
            },
        )
        .await;
        if boot.phase == RuntimePhase::AwaitingOnboarding {
            handle
                .send(UiCommand::CreateMesh {
                    display_name: "P07 Smoke PC".to_owned(),
                })
                .await
                .expect("create mesh");
        }
        wait_for_timeout(
            &handle,
            Duration::from_secs(60),
            |snapshot| snapshot.phase == RuntimePhase::Ready,
        )
        .await;

        handle
            .send(UiCommand::SelectModel {
                reference: ModelReference::qwen3_4b(),
            })
            .await
            .expect("select model");
        wait_for_timeout(&handle, Duration::from_secs(30), |snapshot| {
            snapshot
                .models
                .selected_reference
                .as_ref()
                .is_some_and(|item| item.repository == "Qwen/Qwen3-4B")
        })
        .await;

        handle
            .send(UiCommand::RefreshProviderAccess)
            .await
            .expect("refresh provider access");
        let access = wait_for_timeout(&handle, Duration::from_secs(120), |snapshot| {
            !snapshot.models.busy
                && (snapshot.models.provider_access.status
                    != mesh_core::ProviderAccessStatus::Unchecked
                    || snapshot.models.error.is_some())
        })
        .await;
        assert!(
            access.models.error.is_none(),
            "provider access failed: {:?}",
            access.models.error
        );
        eprintln!(
            "P07 provider access: status={:?} detail={}",
            access.models.provider_access.status,
            access.models.provider_access.detail
        );
        handle
            .send(UiCommand::ProbeSelectedModel)
            .await
            .expect("probe/resolve model");
        let resolved = wait_for_timeout(&handle, Duration::from_secs(30 * 60), |snapshot| {
            !snapshot.models.busy
                && (snapshot.models.resolved_identity.is_some() || snapshot.models.error.is_some())
        })
        .await;
        assert!(
            resolved.models.error.is_none(),
            "model resolve failed: {:?}",
            resolved.models.error
        );
        assert!(
            resolved.models.resolved_identity.is_some(),
            "resolved identity missing: {}",
            resolved.models.status_line
        );

        handle
            .send(UiCommand::PrepareSelectedModel)
            .await
            .expect("prepare model");
        let prepared = wait_for_timeout(
            &handle,
            Duration::from_secs(prepare_timeout_secs),
            |snapshot| {
                !snapshot.models.busy
                    && (snapshot.models.last_prepare_summary.is_some()
                        || snapshot.models.error.is_some())
            },
        )
        .await;
        assert!(
            prepared.models.error.is_none(),
            "model prepare failed: {:?}",
            prepared.models.error
        );
        let prepare_summary = prepared
            .models
            .last_prepare_summary
            .clone()
            .expect("prepare summary");
        eprintln!("P07 prepare: {prepare_summary}");

        handle
            .send(UiCommand::LoadSelectedModel)
            .await
            .expect("load model");
        let loaded = wait_for_timeout(&handle, Duration::from_secs(load_timeout_secs), |snapshot| {
            !snapshot.inference.busy
                && (snapshot.inference.phase == Some(InferencePhase::Ready)
                    || snapshot.inference.phase == Some(InferencePhase::Failed)
                    || snapshot.inference.error.is_some())
        })
        .await;
        assert!(
            loaded.inference.error.is_none(),
            "model load/warmup failed: {:?}",
            loaded.inference.error
        );
        assert_eq!(loaded.inference.phase, Some(InferencePhase::Ready));
        let backend = loaded
            .inference
            .backend
            .clone()
            .unwrap_or_else(|| "unknown".to_owned());
        eprintln!(
            "P07 load ready backend={backend} model={:?}",
            loaded.inference.model_line
        );

        handle
            .send(UiCommand::Generate {
                prompt: "Say hello in one short sentence.".to_owned(),
                max_new_tokens,
                temperature: 0.0,
                seed: 1,
            })
            .await
            .expect("generate");
        let generated = wait_for_timeout(
            &handle,
            Duration::from_secs(generate_timeout_secs),
            |snapshot| {
                !snapshot.inference.busy
                    && (snapshot.inference.generated_tokens > 0
                        || snapshot.inference.phase == Some(InferencePhase::Failed)
                        || snapshot.inference.error.is_some())
            },
        )
        .await;
        assert!(
            generated.inference.error.is_none(),
            "generation failed: {:?}",
            generated.inference.error
        );
        assert!(
            generated.inference.generated_tokens > 0,
            "expected generated tokens, got snapshot {:?}",
            generated.inference
        );
        assert!(
            !generated.inference.output_text.trim().is_empty(),
            "expected non-empty output text"
        );
        assert!(
            !generated.inference.output_text.contains("<think>"),
            "non-thinking profile must not emit open think markers: {:?}",
            generated.inference.output_text
        );
        eprintln!(
            "P07 generate backend={backend} tokens={} stop={:?} output={:?}",
            generated.inference.generated_tokens,
            generated.inference.stop_reason,
            generated.inference.output_text
        );

        handle.request_shutdown().ok();
        let _ = task.await;
    }


    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn p08_remote_replica_generate_smoke() {
        if std::env::var_os("MESH_P08_SMOKE").is_none() {
            eprintln!("skipping P08 remote replica smoke; set MESH_P08_SMOKE=1 to run");
            return;
        }

        let cache_src = std::env::var_os("MESH_P07_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(std::env::temp_dir)
                    .join("mesh-p07-smoke")
            });
        assert!(
            cache_src.join("model-cache").is_dir(),
            "missing prepared cache at {}",
            cache_src.display()
        );

        let root = std::env::temp_dir().join(format!("mesh-p08-{}", now_unix_ms()));
        let worker_dir = root.join("worker");
        let coord_dir = root.join("coord");
        std::fs::create_dir_all(&worker_dir).expect("worker dir");
        std::fs::create_dir_all(&coord_dir).expect("coord dir");
        for dir in [&worker_dir, &coord_dir] {
            let _ = std::os::unix::fs::symlink(
                cache_src.join("model-cache"),
                dir.join("model-cache"),
            );
            let _ = std::os::unix::fs::symlink(cache_src.join("cache"), dir.join("cache"));
        }

        let max_new_tokens = std::env::var("MESH_P08_MAX_NEW_TOKENS")
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(8)
            .max(1);

        let worker = NodeRuntime::create("P08 Worker", StorePaths::isolated(&worker_dir))
            .expect("worker runtime");
        let coord = NodeRuntime::create("P08 Coord", StorePaths::isolated(&coord_dir))
            .expect("coord runtime");
        let worker_handle = worker.handle();
        let coord_handle = coord.handle();
        let worker_task = tokio::spawn(worker.run());
        let coord_task = tokio::spawn(coord.run());

        wait_for(&worker_handle, |snapshot| {
            snapshot.phase == RuntimePhase::AwaitingOnboarding
        })
        .await;
        worker_handle
            .send(UiCommand::CreateMesh {
                display_name: "P08 Worker".to_owned(),
            })
            .await
            .expect("create mesh");
        wait_for(&worker_handle, |snapshot| snapshot.phase == RuntimePhase::Ready).await;
        worker_handle
            .send(UiCommand::CreateInvitation)
            .await
            .expect("invite");
        let invitation = wait_for(&worker_handle, |snapshot| {
            snapshot.enrollment.invitation_text.is_some()
        })
        .await
        .enrollment
        .invitation_text
        .expect("invitation");

        wait_for(&coord_handle, |snapshot| {
            snapshot.phase == RuntimePhase::AwaitingOnboarding
        })
        .await;
        coord_handle
            .send(UiCommand::SubmitInvitation { text: invitation })
            .await
            .expect("join");
        wait_for(&coord_handle, |snapshot| {
            snapshot.phase == RuntimePhase::Ready && !snapshot.peers.is_empty()
        })
        .await;
        wait_for(&worker_handle, |snapshot| !snapshot.peers.is_empty()).await;

        for handle in [&worker_handle, &coord_handle] {
            handle
                .send(UiCommand::SelectModel {
                    reference: ModelReference::qwen3_4b(),
                })
                .await
                .expect("select");
            handle
                .send(UiCommand::RefreshProviderAccess)
                .await
                .expect("access");
            wait_for_timeout(handle, Duration::from_secs(120), |snapshot| {
                !snapshot.models.busy
                    && snapshot.models.provider_access.status
                        != mesh_core::ProviderAccessStatus::Unchecked
            })
            .await;
            handle
                .send(UiCommand::ProbeSelectedModel)
                .await
                .expect("probe");
            wait_for_timeout(handle, Duration::from_secs(30 * 60), |snapshot| {
                !snapshot.models.busy && snapshot.models.resolved_identity.is_some()
            })
            .await;
            handle
                .send(UiCommand::PrepareSelectedModel)
                .await
                .expect("prepare");
            wait_for_timeout(handle, Duration::from_secs(30 * 60), |snapshot| {
                !snapshot.models.busy && snapshot.models.last_prepare_summary.is_some()
            })
            .await;
        }

        worker_handle
            .send(UiCommand::LoadSelectedModel)
            .await
            .expect("worker load");
        let worker_ready = wait_for_timeout(&worker_handle, Duration::from_secs(60 * 60), |snapshot| {
            !snapshot.inference.busy
                && (snapshot.inference.phase == Some(InferencePhase::Ready)
                    || snapshot.inference.phase == Some(InferencePhase::Failed)
                    || snapshot.inference.error.is_some())
        })
        .await;
        assert!(
            worker_ready.inference.error.is_none(),
            "worker load failed: {:?}",
            worker_ready.inference.error
        );
        assert_eq!(worker_ready.inference.phase, Some(InferencePhase::Ready));

        let replica_seen = wait_for_timeout(&coord_handle, Duration::from_secs(60), |snapshot| {
            snapshot.inference.replicas.iter().any(|replica| {
                !replica.local && replica.can_accept() && replica.model_line.contains("Qwen3-4B")
            })
        })
        .await;
        eprintln!(
            "P08 coord replicas={:?}",
            replica_seen
                .inference
                .replicas
                .iter()
                .map(|item| item.status_line())
                .collect::<Vec<_>>()
        );

        coord_handle
            .send(UiCommand::Generate {
                prompt: "Say hello in one short word.".to_owned(),
                max_new_tokens,
                temperature: 0.0,
                seed: 7,
            })
            .await
            .expect("generate");
        let generated = wait_for_timeout(&coord_handle, Duration::from_secs(2 * 60 * 60), |snapshot| {
            !snapshot.inference.busy
                && (snapshot.inference.generated_tokens > 0
                    || snapshot.inference.phase == Some(InferencePhase::Failed)
                    || snapshot.inference.error.is_some())
        })
        .await;
        assert!(
            generated.inference.error.is_none(),
            "remote generation failed: {:?}",
            generated.inference.error
        );
        assert!(
            generated.inference.generated_tokens > 0,
            "expected remote tokens: {:?}",
            generated.inference
        );
        assert!(
            generated
                .inference
                .routed_node_id
                .as_ref()
                .is_some_and(|id| !id.is_empty()),
            "expected routed_node_id"
        );
        assert!(
            !generated.inference.output_text.trim().is_empty(),
            "empty remote output"
        );
        eprintln!(
            "P08 remote generate routed={:?} backend={:?} tokens={} stop={:?} output={:?}",
            generated.inference.routed_node_id,
            generated.inference.backend,
            generated.inference.generated_tokens,
            generated.inference.stop_reason,
            generated.inference.output_text
        );

        worker_handle.request_shutdown().ok();
        coord_handle.request_shutdown().ok();
        let _ = worker_task.await;
        let _ = coord_task.await;
        let _ = std::fs::remove_dir_all(&root);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn p09_dual_node_pipeline_generate_smoke() {
        if std::env::var_os("MESH_P09_MULTI_SMOKE").is_none() {
            eprintln!(
                "skipping P09 dual-node pipeline smoke; set MESH_P09_MULTI_SMOKE=1 to run"
            );
            return;
        }

        let cache_src = std::env::var_os("MESH_P07_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(std::env::temp_dir)
                    .join("mesh-p07-smoke")
            });
        assert!(
            cache_src.join("model-cache").is_dir(),
            "missing prepared cache at {}",
            cache_src.display()
        );

        let root = std::env::temp_dir().join(format!("mesh-p09-{}", now_unix_ms()));
        let first_dir = root.join("first");
        let final_dir = root.join("final");
        std::fs::create_dir_all(&first_dir).expect("first dir");
        std::fs::create_dir_all(&final_dir).expect("final dir");
        for dir in [&first_dir, &final_dir] {
            let _ = std::os::unix::fs::symlink(
                cache_src.join("model-cache"),
                dir.join("model-cache"),
            );
            let _ = std::os::unix::fs::symlink(cache_src.join("cache"), dir.join("cache"));
        }

        let max_new_tokens = std::env::var("MESH_P09_MAX_NEW_TOKENS")
            .ok()
            .and_then(|value| value.parse::<u32>().ok())
            .unwrap_or(4)
            .max(1);

        eprintln!("P09 multi: boot runtimes");
        let first = NodeRuntime::create("P09 First", StorePaths::isolated(&first_dir))
            .expect("first runtime");
        let final_node =
            NodeRuntime::create("P09 Final", StorePaths::isolated(&final_dir)).expect("final runtime");
        let first_handle = first.handle();
        let final_handle = final_node.handle();
        let first_task = tokio::spawn(first.run());
        let final_task = tokio::spawn(final_node.run());

        wait_for(&first_handle, |snapshot| {
            snapshot.phase == RuntimePhase::AwaitingOnboarding
        })
        .await;
        first_handle
            .send(UiCommand::CreateMesh {
                display_name: "P09 First".to_owned(),
            })
            .await
            .expect("create mesh");
        wait_for(&first_handle, |snapshot| snapshot.phase == RuntimePhase::Ready).await;
        first_handle
            .send(UiCommand::CreateInvitation)
            .await
            .expect("invite");
        let invitation = wait_for(&first_handle, |snapshot| {
            snapshot.enrollment.invitation_text.is_some()
        })
        .await
        .enrollment
        .invitation_text
        .expect("invitation");

        wait_for(&final_handle, |snapshot| {
            snapshot.phase == RuntimePhase::AwaitingOnboarding
        })
        .await;
        final_handle
            .send(UiCommand::SubmitInvitation { text: invitation })
            .await
            .expect("join");
        wait_for(&final_handle, |snapshot| {
            snapshot.phase == RuntimePhase::Ready && !snapshot.peers.is_empty()
        })
        .await;
        wait_for(&first_handle, |snapshot| !snapshot.peers.is_empty()).await;
        eprintln!("P09 multi: peers connected");

        for handle in [&first_handle, &final_handle] {
            handle
                .send(UiCommand::SelectModel {
                    reference: ModelReference::qwen3_4b(),
                })
                .await
                .expect("select");
            handle
                .send(UiCommand::RefreshProviderAccess)
                .await
                .expect("access");
            wait_for_timeout(handle, Duration::from_secs(120), |snapshot| {
                !snapshot.models.busy
                    && snapshot.models.provider_access.status
                        != mesh_core::ProviderAccessStatus::Unchecked
            })
            .await;
            handle
                .send(UiCommand::ProbeSelectedModel)
                .await
                .expect("probe");
            wait_for_timeout(handle, Duration::from_secs(30 * 60), |snapshot| {
                !snapshot.models.busy && snapshot.models.resolved_identity.is_some()
            })
            .await;
        }
        eprintln!("P09 multi: probe done (stage prepare deferred to LoadPipelineStage)");

        let first_ready = first_handle.snapshot();
        let final_ready = final_handle.snapshot();
        let first_node_id = first_ready
            .local
            .node_id
            .expect("first node id")
            .to_string();
        let final_node_id = final_ready
            .local
            .node_id
            .expect("final node id")
            .to_string();
        let deployment_id = DeploymentId::new().to_string();
        let node_ids = vec![first_node_id.clone(), final_node_id.clone()];
        let placement = PlacementPlan::split_even(
            DeploymentId::parse_hex(&deployment_id).expect("deployment"),
            "Qwen/Qwen3-4B",
            36,
            &[
                NodeId::parse_hex(&first_node_id).expect("first id"),
                NodeId::parse_hex(&final_node_id).expect("final id"),
            ],
        )
        .expect("placement");
        let first_assignment = placement.stages[0].clone();
        let final_assignment = placement.stages[1].clone();
        eprintln!(
            "P09 multi: placement first={}..{} final={}..{}",
            first_assignment.layer_range.start,
            first_assignment.layer_range.end,
            final_assignment.layer_range.start,
            final_assignment.layer_range.end
        );

        eprintln!("P09 multi: loading first stage");
        first_handle
            .send(UiCommand::LoadPipelineStage {
                deployment_id: deployment_id.clone(),
                model_line: "Qwen/Qwen3-4B".to_owned(),
                num_layers: 36,
                stage_index: first_assignment.stage_index,
                role: first_assignment.role,
                layer_start: first_assignment.layer_range.start,
                layer_end: first_assignment.layer_range.end,
                node_ids: node_ids.clone(),
            })
            .await
            .expect("load first stage");
        let first_loaded =
            wait_for_timeout(&first_handle, Duration::from_secs(20 * 60), |snapshot| {
                !snapshot.inference.busy
                    && (snapshot.inference.phase == Some(InferencePhase::Ready)
                        || snapshot.inference.phase == Some(InferencePhase::Failed)
                        || snapshot.inference.error.is_some())
            })
            .await;
        assert!(
            first_loaded.inference.error.is_none(),
            "first stage load failed: {:?}",
            first_loaded.inference.error
        );
        eprintln!(
            "P09 multi: first stage ready backend={:?}",
            first_loaded.inference.backend
        );

        eprintln!("P09 multi: loading final stage");
        final_handle
            .send(UiCommand::LoadPipelineStage {
                deployment_id: deployment_id.clone(),
                model_line: "Qwen/Qwen3-4B".to_owned(),
                num_layers: 36,
                stage_index: final_assignment.stage_index,
                role: final_assignment.role,
                layer_start: final_assignment.layer_range.start,
                layer_end: final_assignment.layer_range.end,
                node_ids,
            })
            .await
            .expect("load final stage");
        let final_loaded =
            wait_for_timeout(&final_handle, Duration::from_secs(20 * 60), |snapshot| {
                !snapshot.inference.busy
                    && (snapshot.inference.phase == Some(InferencePhase::Ready)
                        || snapshot.inference.phase == Some(InferencePhase::Failed)
                        || snapshot.inference.error.is_some())
            })
            .await;
        assert!(
            final_loaded.inference.error.is_none(),
            "final stage load failed: {:?}",
            final_loaded.inference.error
        );
        assert_eq!(final_loaded.inference.phase, Some(InferencePhase::Ready));
        eprintln!(
            "P09 multi: final stage ready backend={:?}",
            final_loaded.inference.backend
        );

        eprintln!("P09 multi: generate max_new_tokens={max_new_tokens}");
        first_handle
            .send(UiCommand::Generate {
                prompt: "Say hi".to_owned(),
                max_new_tokens,
                temperature: 0.0,
                seed: 7,
            })
            .await
            .expect("generate");
        let generated =
            wait_for_timeout(&first_handle, Duration::from_secs(30 * 60), |snapshot| {
                !snapshot.inference.busy
                    && (snapshot.inference.generated_tokens > 0
                        || snapshot.inference.phase == Some(InferencePhase::Failed)
                        || snapshot.inference.error.is_some())
            })
            .await;
        assert!(
            generated.inference.error.is_none(),
            "pipeline generation failed: {:?}",
            generated.inference.error
        );
        assert!(
            generated.inference.generated_tokens > 0,
            "expected pipeline tokens: {:?}",
            generated.inference
        );
        assert!(
            !generated.inference.output_text.trim().is_empty(),
            "empty pipeline output"
        );
        eprintln!(
            "P09 dual-node pipeline backend={:?} tokens={} stop={:?} output={:?}",
            generated.inference.backend,
            generated.inference.generated_tokens,
            generated.inference.stop_reason,
            generated.inference.output_text
        );

        first_handle.request_shutdown().ok();
        final_handle.request_shutdown().ok();
        let _ = first_task.await;
        let _ = final_task.await;
        let _ = std::fs::remove_dir_all(&root);
    }


    async fn wait_for(
        handle: &NodeHandle,
        predicate: impl Fn(&UiSnapshot) -> bool,
    ) -> UiSnapshot {
        wait_for_timeout(handle, Duration::from_secs(60), predicate).await
    }

    async fn wait_for_timeout(
        handle: &NodeHandle,
        timeout: Duration,
        predicate: impl Fn(&UiSnapshot) -> bool,
    ) -> UiSnapshot {
        let mut snapshots = handle.subscribe_snapshots();
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let snapshot = snapshots.borrow().clone();
            if predicate(&snapshot) {
                return snapshot;
            }
            if tokio::time::Instant::now() >= deadline {
                panic!(
                    "timeout after {timeout:?} waiting for snapshot: phase={:?} status={} models={} inference={:?}",
                    snapshot.phase,
                    snapshot.status_message,
                    snapshot.models.status_line,
                    snapshot.inference
                );
            }
            match tokio::time::timeout(Duration::from_millis(200), snapshots.changed()).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => panic!("snapshot channel closed"),
                Err(_) => {}
            }
        }
    }
}
