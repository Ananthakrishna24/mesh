use std::collections::HashMap;
use std::fmt::{Display, Formatter, Result as FmtResult};
use std::net::SocketAddr;
use std::path::PathBuf;

use mesh_core::invite::{
    build_invite, candidates_from_proto, decode_invitation_text, encode_invitation_text,
};
use mesh_core::protocol::proto::ErrorCode;
use mesh_core::{
    AppScreen, CapabilityReport, CoreError, EndpointCandidate, EnrollmentId, EnrollmentProgress,
    HardwareSummaryView, LinkMeasurement, LocalIdentity, LocalNodeSummary, MeshId, NodeId,
    PeerRecord, PeerSummary, RuntimePhase, UiCommand, UiSnapshot, now_unix_ms,
};
use mesh_hardware::discover_capabilities;
use mesh_net::{
    EnrollmentHello, IncomingPeer, MeshEndpoint, SessionEvent, collect_local_candidates,
    complete_inviter_handshake, generate_node_certificate, perform_joiner_handshake,
    run_connected_session,
};
use mesh_store::{Store, StorePaths};
use rand::RngCore;
use tokio::sync::{broadcast, mpsc, watch};
use tracing::{info, warn};

const COMMAND_CAPACITY: usize = 64;
const EVENT_CAPACITY: usize = 64;
const INVITE_TTL_MS: i64 = 30 * 60 * 1000;

#[derive(Debug)]
pub enum RuntimeError {
    Core(CoreError),
    Store(String),
    Net(String),
    CommandQueueClosed,
    AlreadyShutdown,
}

impl Display for RuntimeError {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::Core(error) => write!(f, "{error}"),
            Self::Store(message) | Self::Net(message) => write!(f, "{message}"),
            Self::CommandQueueClosed => write!(f, "command queue closed"),
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
    PeerJoined { peer: PeerRecord, address: SocketAddr },
    PeerFailed { message: String },
    Session(SessionEvent),
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
    endpoint: Option<MeshEndpoint>,
    paths: StorePaths,
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
        if let Err(error) = self.bootstrap_existing() {
            self.state.set_error(error.to_string());
            self.publish();
        } else if self.state.identity.is_none() {
            let name = self.state.snapshot.local.display_name.clone();
            let hardware = self.state.snapshot.hardware.clone();
            self.state.snapshot = UiSnapshot::first_run(name);
            self.state.snapshot.hardware = hardware;
            self.publish();
        }

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
                shutdown = self.shutdown_rx.recv() => {
                    match shutdown {
                        Ok(()) | Err(broadcast::error::RecvError::Closed) => break,
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    }
                }
            }
        }

        let _ = self.shutdown_tx.send(());
        if let Some(endpoint) = self.endpoint.take() {
            endpoint.close();
        }
        self.state.snapshot.phase = RuntimePhase::ShuttingDown;
        self.state.set_status("Shutting down…");
        self.publish();
        info!("node runtime stopped");
    }

    fn bootstrap_existing(&mut self) -> Result<(), RuntimeError> {
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
        self.start_endpoint(identity)?;
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
                if let Err(error) = self.create_mesh(display_name) {
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
                if let Err(error) = self.join_with_invitation(text).await {
                    self.state.set_error(error.to_string());
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
            UiCommand::Shutdown => true,
        }
    }

    async fn handle_event(&mut self, event: RuntimeEvent) {
        match event {
            RuntimeEvent::Incoming { incoming } => {
                if let Err(error) = self.handle_incoming(incoming).await {
                    warn!(%error, "incoming handshake failed");
                }
            }
            RuntimeEvent::PeerJoined { peer, address } => {
                self.state.upsert_connected_peer(&peer, Some(address));
                self.state
                    .set_status(format!("Connected to {}.", peer.display_name));
                if self.state.snapshot.screen != AppScreen::Dashboard {
                    self.state.set_ready("This PC is ready.");
                } else {
                    self.state.snapshot.phase = RuntimePhase::Ready;
                    self.state.snapshot.can_create_invitation = true;
                }
                self.publish();
            }
            RuntimeEvent::PeerFailed { message } => {
                warn!(%message, "peer task failed");
            }
            RuntimeEvent::Session(session) => self.handle_session_event(session),
        }
    }

    fn handle_session_event(&mut self, event: SessionEvent) {
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
            SessionEvent::Failed {
                peer_node_id,
                message,
            } => {
                warn!(%peer_node_id, %message, "peer session failed");
            }
        }
    }

    async fn handle_incoming(&mut self, incoming: IncomingPeer) -> Result<(), RuntimeError> {
        let identity = self
            .state
            .identity
            .clone()
            .ok_or_else(|| RuntimeError::Store("missing local identity".to_owned()))?;
        let local_candidates = self.state.candidates.clone();
        let known_peers = self.state.known_peers.clone();
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

        self.state.upsert_connected_peer(&peer, Some(remote_address));
        self.state
            .set_status(format!("Connected to {}.", peer.display_name));
        self.state.snapshot.phase = RuntimePhase::Ready;
        self.state.snapshot.can_create_invitation = true;
        self.publish();

        self.spawn_session(identity, peer.node_id, connection, send, recv, hardware);
        Ok(())
    }

    fn create_mesh(&mut self, display_name: String) -> Result<(), RuntimeError> {
        if self.state.identity.is_some() {
            return Err(RuntimeError::Store(
                "this PC is already part of a mesh".to_owned(),
            ));
        }

        self.state.snapshot.phase = RuntimePhase::Preparing;
        self.state.snapshot.enrollment.error = None;
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
        self.start_endpoint(identity)?;
        self.state.push_step("Opened the local connection port");
        self.state
            .set_ready("This PC is ready. Add another PC to enroll a peer.");
        Ok(())
    }

    async fn join_with_invitation(&mut self, text: String) -> Result<(), RuntimeError> {
        if self.state.identity.is_some() {
            return Err(RuntimeError::Store(
                "this PC is already part of a mesh".to_owned(),
            ));
        }

        self.state.snapshot.screen = AppScreen::Enroll;
        self.state.snapshot.phase = RuntimePhase::Preparing;
        self.state.snapshot.enrollment.error = None;
        self.state.snapshot.enrollment.steps.clear();
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
        let candidates = candidates_from_proto(&invite.candidates).map_err(RuntimeError::Core)?;

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
        self.start_endpoint(identity.clone())?;
        self.state.push_step("Opened the local connection port");
        self.state.snapshot.phase = RuntimePhase::Connecting;
        self.state
            .push_step(format!("Connecting to {}", invite.inviter_name));
        self.publish();

        let endpoint = self
            .endpoint
            .as_mut()
            .ok_or_else(|| RuntimeError::Net("endpoint missing".to_owned()))?;
        let local_candidates = self.state.candidates.clone();
        let hardware = self
            .state
            .hardware
            .clone()
            .unwrap_or_else(discover_capabilities);

        let mut last_error = RuntimeError::Net("no invitation candidates succeeded".to_owned());
        for candidate in candidates {
            match endpoint.connect(candidate.address, inviter_node_id).await {
                Ok(peer_connection) => {
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
                        Err(error) => last_error = RuntimeError::Net(error.to_string()),
                    }
                }
                Err(error) => last_error = RuntimeError::Net(error.to_string()),
            }
        }

        Err(last_error)
    }

    fn create_invitation(&mut self) -> Result<(), RuntimeError> {
        let identity = self
            .state
            .identity
            .clone()
            .ok_or_else(|| RuntimeError::Store("create a mesh before inviting peers".to_owned()))?;
        if self.state.candidates.is_empty() {
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
            &self.state.candidates,
        )
        .map_err(RuntimeError::Core)?;
        let text = encode_invitation_text(&invite).map_err(RuntimeError::Core)?;
        self.state.snapshot.enrollment.invitation_text = Some(text);
        self.state
            .set_status("Invitation ready. Copy it to the new PC.");
        Ok(())
    }

    fn start_endpoint(&mut self, identity: LocalIdentity) -> Result<(), RuntimeError> {
        let endpoint = MeshEndpoint::bind(identity.clone(), SocketAddr::from(([0, 0, 0, 0], 0)))
            .map_err(|error| RuntimeError::Net(error.to_string()))?;
        let listen_addr = endpoint.listen_addr();
        self.state.apply_identity(identity, listen_addr);
        self.spawn_accept_loop(endpoint.clone());
        self.endpoint = Some(endpoint);
        Ok(())
    }

    fn spawn_accept_loop(&self, endpoint: MeshEndpoint) {
        let event_tx = self.event_tx.clone();
        let mut shutdown_rx = self.shutdown_tx.subscribe();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown_rx.recv() => break,
                    accepted = endpoint.accept() => {
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
        &self,
        identity: LocalIdentity,
        peer_node_id: NodeId,
        connection: quinn::Connection,
        send: quinn::SendStream,
        recv: quinn::RecvStream,
        hardware: CapabilityReport,
    ) {
        let event_tx = self.event_tx.clone();
        tokio::spawn(async move {
            let (session_tx, mut session_rx) = mpsc::channel(EVENT_CAPACITY);
            let forward = tokio::spawn(async move {
                while let Some(event) = session_rx.recv().await {
                    if event_tx
                        .send(RuntimeEvent::Session(event))
                        .await
                        .is_err()
                    {
                        break;
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
            )
            .await;
            let _ = forward.await;
        });
    }

    fn spawn_reconnect(&self, peer: PeerRecord) {
        let Some(identity) = self.state.identity.clone() else {
            return;
        };
        let local_candidates = self.state.candidates.clone();
        let hardware = self
            .state
            .hardware
            .clone()
            .unwrap_or_else(discover_capabilities);
        let event_tx = self.event_tx.clone();

        tokio::spawn(async move {
            let bind = SocketAddr::from(([0, 0, 0, 0], 0));
            let mut endpoint = match MeshEndpoint::bind(identity.clone(), bind) {
                Ok(endpoint) => endpoint,
                Err(error) => {
                    let _ = event_tx
                        .send(RuntimeEvent::PeerFailed {
                            message: error.to_string(),
                        })
                        .await;
                    return;
                }
            };

            for candidate in peer.candidates {
                if let Ok(connection) = endpoint.connect(candidate.address, peer.node_id).await {
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
                            let _ = event_tx
                                .send(RuntimeEvent::PeerJoined {
                                    peer: welcome.responder.clone(),
                                    address: remote,
                                })
                                .await;
                            let (session_tx, mut session_rx) = mpsc::channel(EVENT_CAPACITY);
                            let forward_tx = event_tx.clone();
                            let forward = tokio::spawn(async move {
                                while let Some(event) = session_rx.recv().await {
                                    if forward_tx
                                        .send(RuntimeEvent::Session(event))
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
                            )
                            .await;
                            let _ = forward.await;
                            return;
                        }
                        Err(error) => warn!(%error, "reconnect handshake failed"),
                    }
                }
            }
            endpoint.close();
        });
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
                && snapshot.hardware.as_ref().is_some_and(|hw| hw.cpu_logical_cores >= 1)
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
