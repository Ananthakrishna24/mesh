use std::fmt::{Display, Formatter, Result as FmtResult};

pub type CoreResult<T> = Result<T, CoreError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoreError {
    InvalidNodeId(String),
    InvalidMeshId(String),
    InvalidEnrollmentId(String),
    InvalidInvitation(String),
    Protocol(String),
    ChannelClosed,
}

impl Display for CoreError {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::InvalidNodeId(value) => write!(f, "invalid node id: {value}"),
            Self::InvalidMeshId(value) => write!(f, "invalid mesh id: {value}"),
            Self::InvalidEnrollmentId(value) => write!(f, "invalid enrollment id: {value}"),
            Self::InvalidInvitation(value) => write!(f, "invalid invitation: {value}"),
            Self::Protocol(value) => write!(f, "protocol error: {value}"),
            Self::ChannelClosed => write!(f, "runtime channel closed"),
        }
    }
}

impl std::error::Error for CoreError {}
