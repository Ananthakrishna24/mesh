use serde::{Deserialize, Serialize};

pub const PROVIDER_HUGGINGFACE: &str = "huggingface";
pub const ADAPTER_QWEN3_DENSE: &str = "qwen3-dense";
pub const ADAPTER_QWEN3_DENSE_VERSION: &str = "1.0.0";
pub const DEFAULT_CACHE_MAX_BYTES: u64 = 0;
pub const CACHE_VOLUME_RESERVE_BYTES: u64 = 5 * 1024 * 1024 * 1024;
pub const CACHE_VOLUME_RESERVE_RATIO_NUM: u64 = 5;
pub const CACHE_VOLUME_RESERVE_RATIO_DEN: u64 = 100;
pub const PARTIAL_GRACE_MS: i64 = 30 * 60 * 1000;
pub const RANGE_MERGE_GAP_BYTES: u64 = 64 * 1024;
pub const MAX_SAFETENSORS_HEADER_BYTES: u64 = 100_000_000;
pub const DISK_PREPARE_MARGIN_BYTES: u64 = 64 * 1024 * 1024;
pub const ETAG_REVALIDATE_MS: i64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelFormat {
    Safetensors,
}

impl ModelFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Safetensors => "safetensors",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "safetensors" => Some(Self::Safetensors),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelReference {
    pub provider: String,
    pub repository: String,
    pub revision_hint: String,
}

impl ModelReference {
    pub fn huggingface(repository: impl Into<String>, revision_hint: impl Into<String>) -> Self {
        Self {
            provider: PROVIDER_HUGGINGFACE.to_owned(),
            repository: repository.into(),
            revision_hint: revision_hint.into(),
        }
    }

    pub fn qwen3_4b() -> Self {
        Self::huggingface("Qwen/Qwen3-4B", "main")
    }

    pub fn qwen3_8b() -> Self {
        Self::huggingface("Qwen/Qwen3-8B", "main")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelIdentity {
    pub provider: String,
    pub repository: String,
    pub revision: String,
    pub manifest_hash: String,
    pub model_format: ModelFormat,
    pub quantization: Option<String>,
    pub tokenizer_hash: String,
}

impl ModelIdentity {
    pub fn summary_line(&self) -> String {
        format!(
            "{}@{} ({})",
            self.repository,
            short_revision(&self.revision),
            &self.manifest_hash[..self.manifest_hash.len().min(12)]
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderAuthMode {
    None,
    Session,
    Saved,
}

impl ProviderAuthMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Session => "session",
            Self::Saved => "saved",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "none" => Some(Self::None),
            "session" => Some(Self::Session),
            "saved" => Some(Self::Saved),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderAccessStatus {
    Ready,
    NeedsToken,
    InvalidToken,
    StoreUnavailable,
    Unchecked,
}

impl ProviderAccessStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::NeedsToken => "needs_token",
            Self::InvalidToken => "invalid_token",
            Self::StoreUnavailable => "store_unavailable",
            Self::Unchecked => "unchecked",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "ready" => Some(Self::Ready),
            "needs_token" => Some(Self::NeedsToken),
            "invalid_token" => Some(Self::InvalidToken),
            "store_unavailable" => Some(Self::StoreUnavailable),
            "unchecked" => Some(Self::Unchecked),
            _ => None,
        }
    }

    pub fn is_ready(self) -> bool {
        matches!(self, Self::Ready)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderAccessReport {
    pub provider: String,
    pub checked_at_unix_ms: i64,
    pub auth_mode: ProviderAuthMode,
    pub public_read: bool,
    pub gated_read: bool,
    pub status: ProviderAccessStatus,
    pub detail: String,
}

impl ProviderAccessReport {
    pub fn unchecked_huggingface() -> Self {
        Self {
            provider: PROVIDER_HUGGINGFACE.to_owned(),
            checked_at_unix_ms: 0,
            auth_mode: ProviderAuthMode::None,
            public_read: false,
            gated_read: false,
            status: ProviderAccessStatus::Unchecked,
            detail: "Provider access has not been checked".to_owned(),
        }
    }

    pub fn huggingface_ready_hint(&self) -> bool {
        self.provider == PROVIDER_HUGGINGFACE && self.status.is_ready() && self.public_read
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CacheValidationState {
    Valid,
    Invalid,
    Partial,
}

impl CacheValidationState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Valid => "valid",
            Self::Invalid => "invalid",
            Self::Partial => "partial",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "valid" => Some(Self::Valid),
            "invalid" => Some(Self::Invalid),
            "partial" => Some(Self::Partial),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelManifestRecord {
    pub cache_key: String,
    pub provider: String,
    pub repository: String,
    pub revision: String,
    pub adapter_id: String,
    pub adapter_version: String,
    pub model_format: ModelFormat,
    pub quantization: Option<String>,
    pub manifest_hash: String,
    pub canonical_bytes: Vec<u8>,
    pub created_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCacheEntry {
    pub entry_id: String,
    pub provider: String,
    pub repository: String,
    pub revision: String,
    pub artifact_path: String,
    pub relative_path: String,
    pub byte_length: u64,
    pub range_start: Option<u64>,
    pub range_end: Option<u64>,
    pub etag: Option<String>,
    pub digest_hex: Option<String>,
    pub dtype: Option<String>,
    pub shape_json: Option<String>,
    pub state: CacheValidationState,
    pub reference_count: u32,
    pub pinned: bool,
    pub last_used_at_unix_ms: i64,
    pub created_at_unix_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ModelCacheView {
    pub root: String,
    pub used_bytes: u64,
    pub protected_bytes: u64,
    pub max_bytes: u64,
    pub entry_count: u32,
    pub partial_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ModelDownloadProgress {
    pub artifact_path: String,
    pub bytes_done: u64,
    pub bytes_total: Option<u64>,
    pub phase: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelStoreView {
    pub provider_access: ProviderAccessReport,
    pub cache: ModelCacheView,
    pub selected_model: Option<String>,
    pub selected_reference: Option<ModelReference>,
    pub resolved_identity: Option<ModelIdentity>,
    pub status_line: String,
    pub error: Option<String>,
    pub busy: bool,
    pub progress: Option<ModelDownloadProgress>,
    pub last_prepare_summary: Option<String>,
}

impl Default for ModelStoreView {
    fn default() -> Self {
        Self {
            provider_access: ProviderAccessReport::unchecked_huggingface(),
            cache: ModelCacheView::default(),
            selected_model: None,
            selected_reference: None,
            resolved_identity: None,
            status_line: "Model provider idle".to_owned(),
            error: None,
            busy: false,
            progress: None,
            last_prepare_summary: None,
        }
    }
}

pub fn short_revision(revision: &str) -> &str {
    if revision.len() > 12 {
        &revision[..12]
    } else {
        revision
    }
}

pub fn is_full_commit_sha(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

pub fn volume_reserve_floor(disk_total_bytes: u64) -> u64 {
    let ratio = disk_total_bytes.saturating_mul(CACHE_VOLUME_RESERVE_RATIO_NUM)
        / CACHE_VOLUME_RESERVE_RATIO_DEN;
    CACHE_VOLUME_RESERVE_BYTES.max(ratio)
}

pub fn prepare_disk_margin(required_bytes: u64) -> u64 {
    let one_percent = required_bytes / 100;
    required_bytes.saturating_add(DISK_PREPARE_MARGIN_BYTES.max(one_percent))
}

pub fn manifest_cache_key(
    provider: &str,
    repository: &str,
    revision: &str,
    adapter_id: &str,
    adapter_version: &str,
    model_format: ModelFormat,
    quantization: Option<&str>,
) -> String {
    format!(
        "{provider}:{repository}:{revision}:adapter={adapter_id}@{adapter_version}:fmt={}:quant={}",
        model_format.as_str(),
        quantization.unwrap_or("none")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_commit_sha_rules() {
        assert!(is_full_commit_sha(
            "0123456789abcdef0123456789abcdef01234567"
        ));
        assert!(!is_full_commit_sha("main"));
        assert!(!is_full_commit_sha(
            "0123456789abcdef0123456789abcdef0123456"
        ));
        assert!(!is_full_commit_sha(
            "0123456789ABCDEF0123456789abcdef01234567"
        ));
    }

    #[test]
    fn reserve_floor_uses_max_of_ratio_and_constant() {
        assert_eq!(volume_reserve_floor(0), CACHE_VOLUME_RESERVE_BYTES);
        assert_eq!(
            volume_reserve_floor(200 * 1024 * 1024 * 1024),
            10 * 1024 * 1024 * 1024
        );
    }

    #[test]
    fn manifest_cache_key_stable() {
        let key = manifest_cache_key(
            PROVIDER_HUGGINGFACE,
            "Qwen/Qwen3-8B",
            "0123456789abcdef0123456789abcdef01234567",
            ADAPTER_QWEN3_DENSE,
            ADAPTER_QWEN3_DENSE_VERSION,
            ModelFormat::Safetensors,
            None,
        );
        assert!(key.contains("adapter=qwen3-dense@1.0.0"));
        assert!(key.ends_with("quant=none"));
    }
}
