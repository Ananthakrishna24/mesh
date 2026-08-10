use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use mesh_core::{
    ADAPTER_QWEN3_DENSE, ADAPTER_QWEN3_DENSE_VERSION, ModelFormat, ModelIdentity,
    is_full_commit_sha, manifest_cache_key,
};

use crate::{ModelError, ModelResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TensorRole {
    Embedding,
    Layer,
    FinalNorm,
    LmHead,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TensorRecord {
    pub name: String,
    pub dtype: String,
    pub shape: Vec<u64>,
    pub role: TensorRole,
    pub layer_index: Option<u32>,
    pub artifact_path: String,
    pub absolute_start: u64,
    pub absolute_end: u64,
    pub range_digest_hex: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRecord {
    pub relative_path: String,
    pub size_bytes: Option<u64>,
    pub etag: Option<String>,
    pub digest_hex: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalManifest {
    pub provider: String,
    pub repository: String,
    pub revision: String,
    pub adapter_id: String,
    pub adapter_version: String,
    pub model_format: ModelFormat,
    pub quantization: Option<String>,
    pub architecture: serde_json::Value,
    pub tensors: Vec<TensorRecord>,
    pub artifacts: Vec<ArtifactRecord>,
    pub tokenizer_artifacts: Vec<String>,
    pub tokenizer_hash: String,
    pub memory_estimate_bytes: u64,
}

impl CanonicalManifest {
    pub fn cache_key(&self) -> String {
        manifest_cache_key(
            &self.provider,
            &self.repository,
            &self.revision,
            &self.adapter_id,
            &self.adapter_version,
            self.model_format,
            self.quantization.as_deref(),
        )
    }

    pub fn sorted(mut self) -> Self {
        self.tensors
            .sort_by(|left, right| left.name.cmp(&right.name));
        self.artifacts
            .sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        self.tokenizer_artifacts.sort();
        self
    }
}

pub fn canonical_manifest_bytes(manifest: &CanonicalManifest) -> ModelResult<Vec<u8>> {
    let sorted = manifest.clone().sorted();
    serde_json::to_vec(&sorted).map_err(ModelError::from)
}

pub fn hash_bytes_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    hex::encode(digest)
}

pub fn manifest_hash_hex(manifest: &CanonicalManifest) -> ModelResult<String> {
    Ok(hash_bytes_hex(&canonical_manifest_bytes(manifest)?))
}

pub fn build_manifest_identity(manifest: &CanonicalManifest) -> ModelResult<ModelIdentity> {
    if !is_full_commit_sha(&manifest.revision) {
        return Err(ModelError::Invalid(
            "model identity revision must be a full lowercase commit sha".to_owned(),
        ));
    }
    Ok(ModelIdentity {
        provider: manifest.provider.clone(),
        repository: manifest.repository.clone(),
        revision: manifest.revision.clone(),
        manifest_hash: manifest_hash_hex(manifest)?,
        model_format: manifest.model_format,
        quantization: manifest.quantization.clone(),
        tokenizer_hash: manifest.tokenizer_hash.clone(),
    })
}

pub fn qwen3_dense_adapter_ids() -> (&'static str, &'static str) {
    (ADAPTER_QWEN3_DENSE, ADAPTER_QWEN3_DENSE_VERSION)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesh_core::PROVIDER_HUGGINGFACE;
    use serde_json::json;

    fn sample_manifest() -> CanonicalManifest {
        CanonicalManifest {
            provider: PROVIDER_HUGGINGFACE.to_owned(),
            repository: "Qwen/Qwen3-4B".to_owned(),
            revision: "0123456789abcdef0123456789abcdef01234567".to_owned(),
            adapter_id: ADAPTER_QWEN3_DENSE.to_owned(),
            adapter_version: ADAPTER_QWEN3_DENSE_VERSION.to_owned(),
            model_format: ModelFormat::Safetensors,
            quantization: None,
            architecture: json!({"model_type":"qwen3","num_hidden_layers":36}),
            tensors: vec![
                TensorRecord {
                    name: "model.layers.1.weight".to_owned(),
                    dtype: "BF16".to_owned(),
                    shape: vec![4],
                    role: TensorRole::Layer,
                    layer_index: Some(1),
                    artifact_path: "model.safetensors".to_owned(),
                    absolute_start: 100,
                    absolute_end: 108,
                    range_digest_hex: None,
                },
                TensorRecord {
                    name: "model.embed_tokens.weight".to_owned(),
                    dtype: "BF16".to_owned(),
                    shape: vec![8],
                    role: TensorRole::Embedding,
                    layer_index: None,
                    artifact_path: "model.safetensors".to_owned(),
                    absolute_start: 20,
                    absolute_end: 36,
                    range_digest_hex: None,
                },
            ],
            artifacts: vec![ArtifactRecord {
                relative_path: "model.safetensors".to_owned(),
                size_bytes: Some(200),
                etag: Some("\"abc\"".to_owned()),
                digest_hex: None,
            }],
            tokenizer_artifacts: vec!["tokenizer.json".to_owned()],
            tokenizer_hash: "aa".repeat(32),
            memory_estimate_bytes: 16,
        }
    }

    #[test]
    fn manifest_hash_is_order_independent() {
        let left = sample_manifest();
        let mut right = sample_manifest();
        right.tensors.reverse();
        assert_eq!(
            manifest_hash_hex(&left).unwrap(),
            manifest_hash_hex(&right).unwrap()
        );
        let identity = build_manifest_identity(&left).unwrap();
        assert_eq!(identity.manifest_hash.len(), 64);
        assert!(identity.revision.len() == 40);
    }
}
