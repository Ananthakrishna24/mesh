use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use mesh_core::{
    CacheValidationState, ModelCacheEntry, PARTIAL_GRACE_MS, PROVIDER_HUGGINGFACE, now_unix_ms,
    volume_reserve_floor,
};
use sha2::{Digest, Sha256};

use crate::{ModelError, ModelResult};

pub fn repository_hash(repository: &str) -> String {
    hex::encode(Sha256::digest(repository.as_bytes()))[..16].to_owned()
}

pub fn artifact_path_hash(relative_path: &str) -> String {
    hex::encode(Sha256::digest(relative_path.as_bytes()))[..16].to_owned()
}

pub fn complete_object_rel_path(
    provider: &str,
    repository: &str,
    revision: &str,
    artifact_path: &str,
) -> String {
    format!(
        "objects/{provider}/{}/{revision}/{}",
        repository_hash(repository),
        artifact_path_hash(artifact_path)
    )
}

pub fn range_object_rel_path(
    provider: &str,
    repository: &str,
    revision: &str,
    artifact_path: &str,
    start: u64,
    end: u64,
) -> String {
    format!(
        "ranges/{provider}/{}/{revision}/{}/{start}_{end}",
        repository_hash(repository),
        artifact_path_hash(artifact_path)
    )
}

pub fn complete_entry_id(
    provider: &str,
    repository: &str,
    revision: &str,
    artifact_path: &str,
    digest_hex: Option<&str>,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(provider.as_bytes());
    hasher.update(b"\0");
    hasher.update(repository.as_bytes());
    hasher.update(b"\0");
    hasher.update(revision.as_bytes());
    hasher.update(b"\0");
    hasher.update(artifact_path.as_bytes());
    hasher.update(b"\0");
    if let Some(digest) = digest_hex {
        hasher.update(digest.as_bytes());
    }
    hex::encode(hasher.finalize())
}

pub fn range_entry_id(
    provider: &str,
    repository: &str,
    revision: &str,
    artifact_path: &str,
    start: u64,
    end: u64,
    dtype: &str,
    shape: &[u64],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(provider.as_bytes());
    hasher.update(b"\0");
    hasher.update(repository.as_bytes());
    hasher.update(b"\0");
    hasher.update(revision.as_bytes());
    hasher.update(b"\0");
    hasher.update(artifact_path.as_bytes());
    hasher.update(b"\0");
    hasher.update(start.to_le_bytes());
    hasher.update(end.to_le_bytes());
    hasher.update(dtype.as_bytes());
    hasher.update(b"\0");
    for dim in shape {
        hasher.update(dim.to_le_bytes());
    }
    hex::encode(hasher.finalize())
}

pub fn absolute_cache_path(root: &Path, relative: &str) -> PathBuf {
    root.join(relative)
}

pub fn partial_path(final_path: &Path) -> PathBuf {
    let mut path = final_path.as_os_str().to_owned();
    path.push(".partial");
    PathBuf::from(path)
}

pub fn partial_meta_path(final_path: &Path) -> PathBuf {
    let mut path = final_path.as_os_str().to_owned();
    path.push(".partial.meta");
    PathBuf::from(path)
}

pub fn ensure_parent(path: &Path) -> ModelResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    Ok(())
}

pub fn file_len(path: &Path) -> ModelResult<u64> {
    Ok(fs::metadata(path)?.len())
}

pub fn publish_file(partial: &Path, final_path: &Path) -> ModelResult<()> {
    ensure_parent(final_path)?;
    if final_path.exists() {
        fs::remove_file(final_path)?;
    }
    fs::rename(partial, final_path)?;
    let meta = partial_meta_path(final_path);
    if meta.exists() {
        let _ = fs::remove_file(meta);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PartialMeta {
    pub provider: String,
    pub repository: String,
    pub revision: String,
    pub artifact_path: String,
    pub full_file: bool,
    pub range_start: Option<u64>,
    pub range_end: Option<u64>,
    pub expected_length: Option<u64>,
    pub etag: Option<String>,
    pub next_offset: u64,
    pub updated_at_unix_ms: i64,
}

impl PartialMeta {
    pub fn write(&self, path: &Path) -> ModelResult<()> {
        ensure_parent(path)?;
        let bytes = serde_json::to_vec_pretty(self)?;
        fs::write(path, bytes)?;
        Ok(())
    }

    pub fn read(path: &Path) -> ModelResult<Self> {
        let bytes = fs::read(path)?;
        Ok(serde_json::from_slice(&bytes)?)
    }
}

pub fn cleanup_incomplete(root: &Path, now_ms: i64, force_all: bool) -> ModelResult<u32> {
    if !root.exists() {
        return Ok(0);
    }
    let mut removed = 0u32;
    cleanup_tree(root, root, now_ms, force_all, &mut removed)?;
    Ok(removed)
}

fn cleanup_tree(
    root: &Path,
    dir: &Path,
    now_ms: i64,
    force_all: bool,
    removed: &mut u32,
) -> ModelResult<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            cleanup_tree(root, &path, now_ms, force_all, removed)?;
            if fs::read_dir(&path)?.next().is_none() {
                let _ = fs::remove_dir(&path);
            }
            continue;
        }
        let name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        let is_partial = name.ends_with(".partial");
        let is_meta = name.ends_with(".partial.meta");
        if !is_partial && !is_meta {
            continue;
        }
        let mtime_ms = modified_unix_ms(&path).unwrap_or(0);
        let stale = force_all || now_ms.saturating_sub(mtime_ms) >= PARTIAL_GRACE_MS;
        if !stale {
            continue;
        }
        if is_partial {
            let meta = partial_meta_path_from_partial(&path);
            let _ = fs::remove_file(&path);
            if meta.exists() {
                let _ = fs::remove_file(meta);
            }
            *removed = removed.saturating_add(1);
        } else if is_meta {
            let partial = partial_from_meta(&path);
            if !partial.exists() {
                let _ = fs::remove_file(&path);
                *removed = removed.saturating_add(1);
            }
        }
    }
    Ok(())
}

fn partial_meta_path_from_partial(partial: &Path) -> PathBuf {
    let text = partial.to_string_lossy();
    if let Some(base) = text.strip_suffix(".partial") {
        PathBuf::from(format!("{base}.partial.meta"))
    } else {
        partial.with_extension("partial.meta")
    }
}

fn partial_from_meta(meta: &Path) -> PathBuf {
    let text = meta.to_string_lossy();
    if let Some(base) = text.strip_suffix(".partial.meta") {
        PathBuf::from(format!("{base}.partial"))
    } else {
        meta.with_extension("partial")
    }
}

fn modified_unix_ms(path: &Path) -> ModelResult<i64> {
    let modified = fs::metadata(path)?.modified()?;
    let duration = modified
        .duration_since(UNIX_EPOCH)
        .map_err(|error| ModelError::Invalid(error.to_string()))?;
    Ok(duration.as_millis() as i64)
}

pub fn validate_entry_file(root: &Path, entry: &ModelCacheEntry) -> ModelResult<bool> {
    if entry.state != CacheValidationState::Valid {
        return Ok(false);
    }
    let path = absolute_cache_path(root, &entry.relative_path);
    if !path.exists() {
        return Ok(false);
    }
    let len = file_len(&path)?;
    Ok(len == entry.byte_length && !entry.relative_path.ends_with(".partial"))
}

pub fn should_evict_for_space(
    used_bytes: u64,
    add_bytes: u64,
    max_bytes: u64,
    disk_available: u64,
    disk_total: u64,
) -> bool {
    let projected = used_bytes.saturating_add(add_bytes);
    if max_bytes > 0 && projected > max_bytes {
        return true;
    }
    let reserve = volume_reserve_floor(disk_total);
    disk_available.saturating_sub(add_bytes) < reserve
}

pub fn default_hf_provider() -> &'static str {
    PROVIDER_HUGGINGFACE
}

pub fn now_ms() -> i64 {
    now_unix_ms()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn complete_paths_are_stable() {
        let path = complete_object_rel_path(
            "huggingface",
            "Qwen/Qwen3-4B",
            "0123456789abcdef0123456789abcdef01234567",
            "model.safetensors",
        );
        assert!(path.starts_with("objects/huggingface/"));
        assert!(path.contains('/'));
    }

    #[test]
    fn cleanup_force_removes_partials() {
        let root = std::env::temp_dir().join(format!("mesh-cache-{}", now_unix_ms()));
        fs::create_dir_all(&root).unwrap();
        let partial = root.join("blob.partial");
        fs::write(&partial, b"abc").unwrap();
        let meta = root.join("blob.partial.meta");
        fs::write(&meta, b"{}").unwrap();
        let removed = cleanup_incomplete(&root, now_unix_ms(), true).unwrap();
        assert!(removed >= 1);
        assert!(!partial.exists());
        let _ = fs::remove_dir_all(root);
    }
}
