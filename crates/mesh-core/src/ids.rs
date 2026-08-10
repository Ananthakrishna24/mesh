use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter, Result as FmtResult};
use uuid::Uuid;

use crate::{CoreError, CoreResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(Uuid);

impl NodeId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }

    pub fn as_uuid(self) -> Uuid {
        self.0
    }

    pub fn parse(value: &str) -> CoreResult<Self> {
        Uuid::parse_str(value)
            .map(Self)
            .map_err(|_| CoreError::InvalidNodeId(value.to_owned()))
    }
}

impl Default for NodeId {
    fn default() -> Self {
        Self::new()
    }
}

impl Display for NodeId {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MeshId(Uuid);

impl MeshId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }

    pub fn as_uuid(self) -> Uuid {
        self.0
    }

    pub fn parse(value: &str) -> CoreResult<Self> {
        Uuid::parse_str(value)
            .map(Self)
            .map_err(|_| CoreError::InvalidMeshId(value.to_owned()))
    }
}

impl Default for MeshId {
    fn default() -> Self {
        Self::new()
    }
}

impl Display for MeshId {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}", self.0)
    }
}
