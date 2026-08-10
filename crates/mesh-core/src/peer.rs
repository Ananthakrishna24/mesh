use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter, Result as FmtResult};
use std::net::SocketAddr;

use crate::{EnrollmentId, LinkMeasurement, NodeId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CandidateKind {
    GlobalIpv6,
    PublicIpv4,
    RouterMapping,
    Manual,
    PeerObserved,
    LocalNetwork,
}

impl CandidateKind {
    pub fn priority(self) -> u16 {
        match self {
            Self::GlobalIpv6 => 100,
            Self::PublicIpv4 => 90,
            Self::RouterMapping => 80,
            Self::Manual => 70,
            Self::PeerObserved => 60,
            Self::LocalNetwork => 50,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointCandidate {
    pub kind: CandidateKind,
    pub address: SocketAddr,
    pub priority: u16,
}

impl EndpointCandidate {
    pub fn new(kind: CandidateKind, address: SocketAddr) -> Self {
        Self {
            kind,
            address,
            priority: kind.priority(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerRecord {
    pub node_id: NodeId,
    pub display_name: String,
    pub certificate_der: Vec<u8>,
    pub candidates: Vec<EndpointCandidate>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PeerSummary {
    pub node_id: NodeId,
    pub display_name: String,
    pub connected: bool,
    pub address: Option<SocketAddr>,
    pub hardware_line: Option<String>,
    pub link: Option<LinkMeasurement>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InvitationState {
    Pending,
    Bound,
    Consumed,
    Expired,
}

impl Display for InvitationState {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Bound => write!(f, "bound"),
            Self::Consumed => write!(f, "consumed"),
            Self::Expired => write!(f, "expired"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvitationRecord {
    pub enrollment_id: EnrollmentId,
    pub secret_digest: [u8; 32],
    pub expires_at_unix_ms: i64,
    pub state: InvitationState,
    pub bound_node_id: Option<NodeId>,
    pub created_at_unix_ms: i64,
}
