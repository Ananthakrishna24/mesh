use std::collections::HashMap;
use std::fmt::{Display, Formatter, Result as FmtResult};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use mesh_core::invite::{
    build_invite, candidates_from_proto, decode_invitation_text, encode_invitation_text,
};
use mesh_core::protocol::proto::ErrorCode;
use mesh_core::{
    AppScreen, CandidateKind, CapabilityReport, ConnectivityRecovery, CoreError, EndpointCandidate,
    EnrollmentId, EnrollmentProgress, HardwareSummaryView, LinkMeasurement, LocalIdentity,
    LocalNodeSummary, ManualForwardingGuide, MeshId, NodeId, PeerRecord, PeerRecordOrigin,
    PeerSummary, RecoveryAction, RuntimePhase, UiCommand, UiSnapshot, filter_advertised_candidates,
    merge_peer_records, now_unix_ms, sort_candidates_for_dial,
};
use mesh_hardware::discover_capabilities;
use mesh_net::{
    EnrollmentHello, HOLE_PUNCH_WINDOW, IncomingPeer, MeshEndpoint, RouterMappingHandle,
    SessionCommand, SessionEvent, advertised_candidates, attempt_router_mapping,
    collect_local_candidates, complete_inviter_handshake, generate_node_certificate,
    new_attempt_id, perform_joiner_handshake, run_connected_session, send_udp_probes,
    start_at_after, wait_until_unix_ms, with_manual_candidate, with_peer_observed,
    with_router_mapping,
};
use mesh_store::{Store, StorePaths};
use rand::RngCore;
use tokio::sync::{Mutex, broadcast, mpsc, watch};
use tracing::{info, warn};

const COMMAND_CAPACITY: usize = 64;
const EVENT_CAPACITY: usize = 64;
const INVITE_TTL_MS: i64 = 30 * 60 * 1000;
const PEER_UPDATE_COALESCE: Duration = Duration::from_secs(5);
const SELF_REFRESH_INTERVAL: Duration = Duration::from_secs(10 * 60);
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
                link: previous.and_then(|item| item.link),
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
}

struct LiveSession {
    commands: mpsc::Sender<SessionCommand>,
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
    peer_update_dirty: bool,
    last_peer_update: Option<tokio::time::Instant>,
}

impl NodeRuntime {
    pub fn create(display_name: impl Into<String>, paths: StorePaths) -> Result<Self, RuntimeError> {
        let display_name = display_name.into();
        let store =
            Store::open(paths.clone()).map_err(|error| RuntimeError::Store(error.to_string()))?;
        let mut state = RuntimeState::new(display_name);
        state.set_hardware(discover_capabilities());
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
            peer_update_dirty: false,
            last_peer_update: None,
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
            self.state.snapshot = UiSnapshot::first_run(name);
            self.state.snapshot.hardware = hardware;
            self.publish();
        }

        let mut peer_update_tick = tokio::time::interval(PEER_UPDATE_COALESCE);
        peer_update_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut self_refresh_tick = tokio::time::interval(SELF_REFRESH_INTERVAL);
        self_refresh_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

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
                    self.state.snapshot = UiSnapshot::first_run(name);
                    self.state.snapshot.hardware = hardware;
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
                self.state.set_hardware(discover_capabilities());
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
                    self.state.set_status("Refreshed local connectivity candidates.");
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
            SessionEvent::Failed {
                peer_node_id,
                message,
            } => {
                warn!(%peer_node_id, %message, "peer session failed");
                self.sessions.remove(&peer_node_id);
                if let Some(peer) = self.state.peers.get_mut(&peer_node_id) {
                    peer.connected = false;
                    self.state.rebuild_peer_summaries();
                    self.publish();
                }
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

    fn publish(&self) {
        let _ = self.snapshot_tx.send(self.state.snapshot.clone());
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

    async fn wait_for(
        handle: &NodeHandle,
        predicate: impl Fn(&UiSnapshot) -> bool,
    ) -> UiSnapshot {
        let mut snapshots = handle.subscribe_snapshots();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
        loop {
            let snapshot = snapshots.borrow().clone();
            if predicate(&snapshot) {
                return snapshot;
            }
            if tokio::time::Instant::now() >= deadline {
                panic!("timeout waiting for snapshot: {snapshot:?}");
            }
            match tokio::time::timeout(Duration::from_millis(200), snapshots.changed()).await {
                Ok(Ok(())) => {}
                Ok(Err(_)) => panic!("snapshot channel closed"),
                Err(_) => {}
            }
        }
    }
}
