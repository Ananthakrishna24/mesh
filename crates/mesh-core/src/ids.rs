use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt::{Debug, Display, Formatter, Result as FmtResult};

use crate::{CoreError, CoreResult};

const NODE_ID_LEN: usize = 32;
const MESH_ID_LEN: usize = 16;
const ENROLLMENT_ID_LEN: usize = 16;
const DEPLOYMENT_ID_LEN: usize = 16;
const RESERVATION_ID_LEN: usize = 16;
const REQUEST_ID_LEN: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId([u8; NODE_ID_LEN]);

impl NodeId {
    pub const LEN: usize = NODE_ID_LEN;

    pub fn from_certificate_der(certificate_der: &[u8]) -> Self {
        let digest = Sha256::digest(certificate_der);
        let mut bytes = [0u8; NODE_ID_LEN];
        bytes.copy_from_slice(&digest);
        Self(bytes)
    }

    pub fn from_bytes(bytes: [u8; NODE_ID_LEN]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; NODE_ID_LEN] {
        &self.0
    }

    pub fn to_vec(self) -> Vec<u8> {
        self.0.to_vec()
    }

    pub fn from_slice(bytes: &[u8]) -> CoreResult<Self> {
        let array: [u8; NODE_ID_LEN] = bytes
            .try_into()
            .map_err(|_| CoreError::InvalidNodeId(hex::encode(bytes)))?;
        Ok(Self(array))
    }

    pub fn parse_hex(value: &str) -> CoreResult<Self> {
        let bytes =
            hex::decode(value).map_err(|_| CoreError::InvalidNodeId(value.to_owned()))?;
        Self::from_slice(&bytes)
    }

    pub fn short_hex(self) -> String {
        hex::encode(&self.0[..4])
    }
}

impl Debug for NodeId {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "NodeId({})", self.short_hex())
    }
}

impl Display for NodeId {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}", hex::encode(self.0))
    }
}

impl PartialOrd for NodeId {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for NodeId {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MeshId([u8; MESH_ID_LEN]);

impl MeshId {
    pub const LEN: usize = MESH_ID_LEN;

    pub fn new() -> Self {
        let mut bytes = [0u8; MESH_ID_LEN];
        rand::rng().fill_bytes(&mut bytes);
        Self(bytes)
    }

    pub fn from_bytes(bytes: [u8; MESH_ID_LEN]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; MESH_ID_LEN] {
        &self.0
    }

    pub fn to_vec(self) -> Vec<u8> {
        self.0.to_vec()
    }

    pub fn from_slice(bytes: &[u8]) -> CoreResult<Self> {
        let array: [u8; MESH_ID_LEN] = bytes
            .try_into()
            .map_err(|_| CoreError::InvalidMeshId(hex::encode(bytes)))?;
        Ok(Self(array))
    }

    pub fn parse_hex(value: &str) -> CoreResult<Self> {
        let bytes =
            hex::decode(value).map_err(|_| CoreError::InvalidMeshId(value.to_owned()))?;
        Self::from_slice(&bytes)
    }

    pub fn short_hex(self) -> String {
        hex::encode(&self.0[..4])
    }
}

impl Default for MeshId {
    fn default() -> Self {
        Self::new()
    }
}

impl Debug for MeshId {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "MeshId({})", self.short_hex())
    }
}

impl Display for MeshId {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}", hex::encode(self.0))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EnrollmentId([u8; ENROLLMENT_ID_LEN]);

impl EnrollmentId {
    pub const LEN: usize = ENROLLMENT_ID_LEN;

    pub fn new() -> Self {
        let mut bytes = [0u8; ENROLLMENT_ID_LEN];
        rand::rng().fill_bytes(&mut bytes);
        Self(bytes)
    }

    pub fn from_bytes(bytes: [u8; ENROLLMENT_ID_LEN]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; ENROLLMENT_ID_LEN] {
        &self.0
    }

    pub fn to_vec(self) -> Vec<u8> {
        self.0.to_vec()
    }

    pub fn from_slice(bytes: &[u8]) -> CoreResult<Self> {
        let array: [u8; ENROLLMENT_ID_LEN] = bytes
            .try_into()
            .map_err(|_| CoreError::InvalidEnrollmentId(hex::encode(bytes)))?;
        Ok(Self(array))
    }
}

impl Default for EnrollmentId {
    fn default() -> Self {
        Self::new()
    }
}

impl Debug for EnrollmentId {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "EnrollmentId({})", hex::encode(&self.0[..4]))
    }
}

impl Display for EnrollmentId {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}", hex::encode(self.0))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DeploymentId([u8; DEPLOYMENT_ID_LEN]);

impl DeploymentId {
    pub const LEN: usize = DEPLOYMENT_ID_LEN;

    pub fn new() -> Self {
        let mut bytes = [0u8; DEPLOYMENT_ID_LEN];
        rand::rng().fill_bytes(&mut bytes);
        Self(bytes)
    }

    pub fn from_bytes(bytes: [u8; DEPLOYMENT_ID_LEN]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; DEPLOYMENT_ID_LEN] {
        &self.0
    }

    pub fn to_vec(self) -> Vec<u8> {
        self.0.to_vec()
    }

    pub fn from_slice(bytes: &[u8]) -> CoreResult<Self> {
        let array: [u8; DEPLOYMENT_ID_LEN] = bytes
            .try_into()
            .map_err(|_| CoreError::InvalidDeploymentId(hex::encode(bytes)))?;
        Ok(Self(array))
    }

    pub fn parse_hex(value: &str) -> CoreResult<Self> {
        let bytes = hex::decode(value)
            .map_err(|_| CoreError::InvalidDeploymentId(value.to_owned()))?;
        Self::from_slice(&bytes)
    }

    pub fn short_hex(self) -> String {
        hex::encode(&self.0[..4])
    }
}

impl Default for DeploymentId {
    fn default() -> Self {
        Self::new()
    }
}

impl Debug for DeploymentId {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "DeploymentId({})", self.short_hex())
    }
}

impl Display for DeploymentId {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}", hex::encode(self.0))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RequestId([u8; REQUEST_ID_LEN]);

impl RequestId {
    pub const LEN: usize = REQUEST_ID_LEN;

    pub fn new() -> Self {
        let mut bytes = [0u8; REQUEST_ID_LEN];
        rand::rng().fill_bytes(&mut bytes);
        Self(bytes)
    }

    pub fn from_bytes(bytes: [u8; REQUEST_ID_LEN]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; REQUEST_ID_LEN] {
        &self.0
    }

    pub fn to_vec(self) -> Vec<u8> {
        self.0.to_vec()
    }

    pub fn from_slice(bytes: &[u8]) -> CoreResult<Self> {
        let array: [u8; REQUEST_ID_LEN] = bytes
            .try_into()
            .map_err(|_| CoreError::InvalidRequestId(hex::encode(bytes)))?;
        Ok(Self(array))
    }

    pub fn short_hex(self) -> String {
        hex::encode(&self.0[..4])
    }
}

impl Default for RequestId {
    fn default() -> Self {
        Self::new()
    }
}

impl Debug for RequestId {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "RequestId({})", self.short_hex())
    }
}

impl Display for RequestId {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}", hex::encode(self.0))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ReservationId([u8; RESERVATION_ID_LEN]);

impl ReservationId {
    pub const LEN: usize = RESERVATION_ID_LEN;

    pub fn new() -> Self {
        let mut bytes = [0u8; RESERVATION_ID_LEN];
        rand::rng().fill_bytes(&mut bytes);
        Self(bytes)
    }

    pub fn from_bytes(bytes: [u8; RESERVATION_ID_LEN]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; RESERVATION_ID_LEN] {
        &self.0
    }

    pub fn to_vec(self) -> Vec<u8> {
        self.0.to_vec()
    }

    pub fn from_slice(bytes: &[u8]) -> CoreResult<Self> {
        let array: [u8; RESERVATION_ID_LEN] = bytes
            .try_into()
            .map_err(|_| CoreError::InvalidReservationId(hex::encode(bytes)))?;
        Ok(Self(array))
    }

    pub fn short_hex(self) -> String {
        hex::encode(&self.0[..4])
    }
}

impl Default for ReservationId {
    fn default() -> Self {
        Self::new()
    }
}

impl Debug for ReservationId {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "ReservationId({})", self.short_hex())
    }
}

impl Display for ReservationId {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        write!(f, "{}", hex::encode(self.0))
    }
}
