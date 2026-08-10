use std::fmt::{Display, Formatter, Result as FmtResult};

use mesh_core::{
    AppScreen, CoreError, LocalNodeSummary, MeshId, NodeId, RuntimePhase, UiCommand, UiSnapshot,
};
use tokio::sync::{broadcast, mpsc, watch};
use tracing::{debug, info, warn};

const COMMAND_CAPACITY: usize = 64;
const EVENT_CAPACITY: usize = 64;

#[derive(Debug)]
pub enum RuntimeError {
    Core(CoreError),
    CommandQueueClosed,
    AlreadyShutdown,
}

impl Display for RuntimeError {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::Core(error) => write!(f, "{error}"),
            Self::CommandQueueClosed => write!(f, "command queue closed"),
            Self::AlreadyShutdown => write!(f, "runtime already shut down"),
        }
    }
}

impl std::error::Error for RuntimeError {}

impl From<CoreError> for RuntimeError {
    fn from(value: CoreError) -> Self {
        Self::Core(value)
    }
}

#[derive(Debug)]
struct RuntimeState {
    snapshot: UiSnapshot,
    node_id: Option<NodeId>,
    mesh_id: Option<MeshId>,
}

impl RuntimeState {
    fn new(display_name: String) -> Self {
        Self {
            snapshot: UiSnapshot::starting(display_name),
            node_id: None,
            mesh_id: None,
        }
    }

    fn publish_first_run(&mut self) {
        let display_name = self.snapshot.local.display_name.clone();
        self.snapshot = UiSnapshot::first_run(display_name);
    }

    fn create_mesh(&mut self, display_name: String) {
        let node_id = NodeId::new();
        let mesh_id = MeshId::new();
        self.node_id = Some(node_id);
        self.mesh_id = Some(mesh_id);
        self.snapshot = UiSnapshot {
            screen: AppScreen::Dashboard,
            phase: RuntimePhase::Ready,
            local: LocalNodeSummary {
                display_name,
                node_id: Some(node_id),
                mesh_id: Some(mesh_id),
            },
            peers: Vec::new(),
            status_message: "This PC is ready. Add another PC when enrollment is implemented."
                .to_owned(),
            enrollment_open: false,
        };
    }

    fn open_enrollment(&mut self) {
        self.snapshot.enrollment_open = true;
        self.snapshot.status_message =
            "Enrollment arrives in P02. Paste support is not available yet.".to_owned();
    }

    fn cancel_enrollment(&mut self) {
        self.snapshot.enrollment_open = false;
        if self.snapshot.screen == AppScreen::FirstRun {
            self.snapshot.status_message = "Create a mesh or enroll this PC.".to_owned();
        }
    }

    fn begin_shutdown(&mut self) {
        self.snapshot.phase = RuntimePhase::ShuttingDown;
        self.snapshot.status_message = "Shutting down…".to_owned();
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

pub struct NodeRuntime {
    handle: NodeHandle,
    command_rx: mpsc::Receiver<UiCommand>,
    snapshot_tx: watch::Sender<UiSnapshot>,
    shutdown_rx: broadcast::Receiver<()>,
    state: RuntimeState,
}

impl NodeRuntime {
    pub fn create(display_name: impl Into<String>) -> Self {
        let display_name = display_name.into();
        let state = RuntimeState::new(display_name);
        let (command_tx, command_rx) = mpsc::channel(COMMAND_CAPACITY);
        let (snapshot_tx, snapshot_rx) = watch::channel(state.snapshot.clone());
        let (shutdown_tx, shutdown_rx) = broadcast::channel(EVENT_CAPACITY);

        let handle = NodeHandle {
            commands: command_tx,
            snapshots: snapshot_rx,
            shutdown: shutdown_tx.clone(),
        };

        Self {
            handle,
            command_rx,
            snapshot_tx,
            shutdown_rx,
            state,
        }
    }

    pub fn handle(&self) -> NodeHandle {
        self.handle.clone()
    }

    pub async fn run(mut self) {
        info!("node runtime started");
        self.state.publish_first_run();
        self.publish();

        loop {
            tokio::select! {
                command = self.command_rx.recv() => {
                    match command {
                        Some(command) => {
                            if self.handle_command(command) {
                                break;
                            }
                        }
                        None => {
                            debug!("command channel closed");
                            break;
                        }
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

        self.state.begin_shutdown();
        self.publish();
        info!("node runtime stopped");
    }


    fn handle_command(&mut self, command: UiCommand) -> bool {
        match command {
            UiCommand::CreateMesh { display_name } => {
                let name = normalize_display_name(display_name);
                info!(%name, "creating local mesh shell");
                self.state.create_mesh(name);
                self.publish();
                false
            }
            UiCommand::OpenEnrollment => {
                self.state.open_enrollment();
                self.publish();
                false
            }
            UiCommand::CancelEnrollment => {
                self.state.cancel_enrollment();
                self.publish();
                false
            }
            UiCommand::Shutdown => {
                info!("shutdown requested");
                true
            }
        }
    }

    fn publish(&self) {
        if self.snapshot_tx.send(self.state.snapshot.clone()).is_err() {
            warn!("no snapshot subscribers");
        }
    }
}

fn normalize_display_name(value: String) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        default_display_name()
    } else {
        trimmed.to_owned()
    }
}

fn default_display_name() -> String {
    "This PC".to_owned()
}


#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn create_mesh_publishes_dashboard_snapshot() {
        let runtime = NodeRuntime::create("Test PC");
        let handle = runtime.handle();
        let mut snapshots = handle.subscribe_snapshots();

        let worker = tokio::spawn(runtime.run());

        snapshots
            .wait_for(|snapshot| snapshot.phase == RuntimePhase::AwaitingOnboarding)
            .await
            .expect("first-run snapshot");

        handle
            .send(UiCommand::CreateMesh {
                display_name: "Lab PC".to_owned(),
            })
            .await
            .expect("create mesh");

        let snapshot = snapshots
            .wait_for(|snapshot| snapshot.screen == AppScreen::Dashboard)
            .await
            .expect("dashboard snapshot")
            .clone();

        assert_eq!(snapshot.phase, RuntimePhase::Ready);
        assert_eq!(snapshot.local.display_name, "Lab PC");
        assert!(snapshot.local.node_id.is_some());
        assert!(snapshot.local.mesh_id.is_some());
        assert!(snapshot.peers.is_empty());

        handle.request_shutdown().expect("shutdown");
        worker.await.expect("runtime task");
    }
}