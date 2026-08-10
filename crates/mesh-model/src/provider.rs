use std::path::PathBuf;

use mesh_core::ModelIdentity;
use serde::{Deserialize, Serialize};

use crate::manifest::CanonicalManifest;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub provider: String,
    pub repository: String,
    pub revision: String,
    pub relative_path: String,
    pub size_bytes: Option<u64>,
    pub etag: Option<String>,
    pub digest_hex: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactMetadata {
    pub relative_path: String,
    pub size_bytes: Option<u64>,
    pub etag: Option<String>,
    pub digest_hex: Option<String>,
    pub commit_sha: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedModel {
    pub identity: ModelIdentity,
    pub manifest: CanonicalManifest,
    pub artifacts: Vec<ArtifactRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TensorAssignment {
    pub name: String,
    pub dtype: String,
    pub shape: Vec<u64>,
    pub artifact_path: String,
    pub absolute_start: u64,
    pub absolute_end: u64,
    pub layer_index: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeModelPlan {
    pub deployment_id: String,
    pub model: ModelIdentity,
    pub assignment_hash: String,
    pub first_layer: u32,
    pub last_layer_exclusive: u32,
    pub tensor_assignments: Vec<TensorAssignment>,
    pub global_tensors: Vec<TensorAssignment>,
    pub disk_bytes_required: u64,
    pub gpu_bytes_reserved: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedArtifact {
    pub entry_id: String,
    pub relative_path: PathBuf,
    pub artifact_path: String,
    pub byte_length: u64,
    pub range_start: Option<u64>,
    pub range_end: Option<u64>,
}
