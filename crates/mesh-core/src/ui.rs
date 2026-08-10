use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

use crate::{MeshId, NodeId, PeerSummary};

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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UiCommand {
    CreateMesh { display_name: String },
    OpenEnrollment,
    CancelEnrollment,
    SubmitInvitation { text: String },
    CreateInvitation,
    ClearInvitation,
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiSnapshot {
    pub screen: AppScreen,
    pub phase: RuntimePhase,
    pub local: LocalNodeSummary,
    pub peers: Vec<PeerSummary>,
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
            status_message: "Starting local runtime…".to_owned(),
            enrollment: EnrollmentProgress {
                steps: Vec::new(),
                current: String::new(),
                invitation_text: None,
                error: None,
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
