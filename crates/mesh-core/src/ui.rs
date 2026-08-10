use serde::{Deserialize, Serialize};

use crate::{MeshId, NodeId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppScreen {
    FirstRun,
    Dashboard,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RuntimePhase {
    Starting,
    AwaitingOnboarding,
    Ready,
    ShuttingDown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalNodeSummary {
    pub display_name: String,
    pub node_id: Option<NodeId>,
    pub mesh_id: Option<MeshId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerSummary {
    pub node_id: NodeId,
    pub display_name: String,
    pub connected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum UiCommand {
    CreateMesh { display_name: String },
    OpenEnrollment,
    CancelEnrollment,
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiSnapshot {
    pub screen: AppScreen,
    pub phase: RuntimePhase,
    pub local: LocalNodeSummary,
    pub peers: Vec<PeerSummary>,
    pub status_message: String,
    pub enrollment_open: bool,
}

impl UiSnapshot {
    pub fn starting(display_name: impl Into<String>) -> Self {
        let display_name = display_name.into();
        Self {
            screen: AppScreen::FirstRun,
            phase: RuntimePhase::Starting,
            local: LocalNodeSummary {
                display_name,
                node_id: None,
                mesh_id: None,
            },
            peers: Vec::new(),
            status_message: "Starting local runtime…".to_owned(),
            enrollment_open: false,
        }
    }

    pub fn first_run(display_name: impl Into<String>) -> Self {
        let display_name = display_name.into();
        Self {
            screen: AppScreen::FirstRun,
            phase: RuntimePhase::AwaitingOnboarding,
            local: LocalNodeSummary {
                display_name,
                node_id: None,
                mesh_id: None,
            },
            peers: Vec::new(),
            status_message: "Create a mesh or enroll this PC.".to_owned(),
            enrollment_open: false,
        }
    }
}
