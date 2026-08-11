use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::Duration;

use bytes::Bytes;
use futures_util::StreamExt;
use hf_hub::api::tokio::{Api, ApiBuilder};
use hf_hub::{Repo, RepoType};
use mesh_core::{
    ModelIdentity, ModelReference, PROVIDER_HUGGINGFACE, ProviderAccessReport,
    ProviderAccessStatus, ProviderAuthMode, is_full_commit_sha, now_unix_ms,
};
use reqwest::header::{AUTHORIZATION, CONTENT_LENGTH, CONTENT_RANGE, ETAG, RANGE};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::adapter::{AdapterInputs, WeightShard, build_qwen3_dense_manifest};
use crate::manifest::{ArtifactRecord, build_manifest_identity, hash_bytes_hex};
use crate::provider::{ArtifactMetadata, ArtifactRef, ResolvedModel};
use crate::safetensors::{parse_header_length, parse_safetensors_header};
use crate::validate::{RangeValidation, parse_content_range_header, validate_content_range};
use crate::{ModelError, ModelResult};

const USER_AGENT: &str = "mesh/0.1 (+https://github.com/local/mesh)";
const MAX_HEADER_FETCH: u64 = 8 + 8 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct HuggingFaceProvider {
    token: Option<String>,
    auth_mode: ProviderAuthMode,
    endpoint: String,
    client: Client,
    meta_client: Client,
    hf_cache_dir: PathBuf,
}

impl HuggingFaceProvider {
    pub fn new(
        token: Option<String>,
        auth_mode: ProviderAuthMode,
        hf_cache_dir: impl Into<PathBuf>,
    ) -> ModelResult<Self> {
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(3 * 60 * 60))
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()
            .map_err(|error| ModelError::Http(error.to_string()))?;
        // Metadata must observe origin headers before CDN redirect. Hugging Face
        // puts the LFS content SHA in `x-linked-etag`; the CDN ETag is not that digest.
        let meta_client = Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| ModelError::Http(error.to_string()))?;
        Ok(Self {
            token: token.and_then(|value| {
                let trimmed = value.trim().to_owned();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed)
                }
            }),
            auth_mode,
            endpoint: std::env::var("HF_ENDPOINT")
                .unwrap_or_else(|_| "https://huggingface.co".to_owned()),
            client,
            meta_client,
            hf_cache_dir: hf_cache_dir.into(),
        })
    }

    pub fn with_token(mut self, token: Option<String>, auth_mode: ProviderAuthMode) -> Self {
        self.token = token.and_then(|value| {
            let trimmed = value.trim().to_owned();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        });
        self.auth_mode = auth_mode;
        self
    }

    fn api(&self) -> ModelResult<Api> {
        let builder = ApiBuilder::new()
            .with_cache_dir(self.hf_cache_dir.clone())
            .with_endpoint(self.endpoint.clone())
            .with_progress(false)
            .with_token(self.token.clone());
        builder
            .build()
            .map_err(|error| ModelError::Provider(error.to_string()))
    }

    fn authorize(&self, request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(token) = &self.token {
            request.header(AUTHORIZATION, format!("Bearer {token}"))
        } else {
            request
        }
    }

    fn resolve_url(&self, repository: &str, revision: &str, relative_path: &str) -> String {
        let revision = revision.replace('/', "%2F");
        format!(
            "{}/{}",
            self.endpoint.trim_end_matches('/'),
            format!("{repository}/resolve/{revision}/{relative_path}")
        )
    }

    pub async fn probe_access(
        &self,
        reference: &ModelReference,
    ) -> ModelResult<ProviderAccessReport> {
        if reference.provider != PROVIDER_HUGGINGFACE {
            return Err(ModelError::Unsupported(format!(
                "provider {} is not huggingface",
                reference.provider
            )));
        }
        let revision_hint = if reference.revision_hint.trim().is_empty() {
            "main"
        } else {
            reference.revision_hint.trim()
        };
        let api = self.api()?;
        let repo = api.repo(Repo::with_revision(
            reference.repository.clone(),
            RepoType::Model,
            revision_hint.to_owned(),
        ));
        match repo.info().await {
            Ok(_) => Ok(ProviderAccessReport {
                provider: PROVIDER_HUGGINGFACE.to_owned(),
                checked_at_unix_ms: now_unix_ms(),
                auth_mode: self.auth_mode,
                public_read: self.token.is_none() || self.auth_mode == ProviderAuthMode::None,
                gated_read: self.token.is_some(),
                status: ProviderAccessStatus::Ready,
                detail: if self.token.is_some() {
                    "Hugging Face access verified with saved token".to_owned()
                } else {
                    "Hugging Face public metadata access verified".to_owned()
                },
            }),
            Err(error) => {
                let text = error.to_string();
                let lower = text.to_ascii_lowercase();
                if lower.contains("401") || lower.contains("unauthorized") {
                    Ok(ProviderAccessReport {
                        provider: PROVIDER_HUGGINGFACE.to_owned(),
                        checked_at_unix_ms: now_unix_ms(),
                        auth_mode: self.auth_mode,
                        public_read: false,
                        gated_read: false,
                        status: if self.token.is_some() {
                            ProviderAccessStatus::InvalidToken
                        } else {
                            ProviderAccessStatus::NeedsToken
                        },
                        detail: "Hugging Face rejected credentials for this repository".to_owned(),
                    })
                } else if lower.contains("403") || lower.contains("gated") {
                    Ok(ProviderAccessReport {
                        provider: PROVIDER_HUGGINGFACE.to_owned(),
                        checked_at_unix_ms: now_unix_ms(),
                        auth_mode: self.auth_mode,
                        public_read: false,
                        gated_read: false,
                        status: if self.token.is_some() {
                            ProviderAccessStatus::InvalidToken
                        } else {
                            ProviderAccessStatus::NeedsToken
                        },
                        detail: "Repository is gated or private; save a valid read token"
                            .to_owned(),
                    })
                } else {
                    Err(ModelError::Provider(text))
                }
            }
        }
    }

    pub async fn resolve(&self, reference: &ModelReference) -> ModelResult<ResolvedModel> {
        if reference.provider != PROVIDER_HUGGINGFACE {
            return Err(ModelError::Unsupported(format!(
                "provider {} is not supported",
                reference.provider
            )));
        }
        let revision_hint = if reference.revision_hint.trim().is_empty() {
            "main".to_owned()
        } else {
            reference.revision_hint.trim().to_owned()
        };

        let api = self.api()?;
        let repo = api.repo(Repo::with_revision(
            reference.repository.clone(),
            RepoType::Model,
            revision_hint,
        ));
        let info = repo
            .info()
            .await
            .map_err(|error| classify_provider_error(error.to_string()))?;
        let revision = info.sha.to_ascii_lowercase();
        if !is_full_commit_sha(&revision) {
            return Err(ModelError::Invalid(format!(
                "provider returned non-sha revision {}",
                info.sha
            )));
        }

        let names = info
            .siblings
            .iter()
            .map(|item| item.rfilename.clone())
            .collect::<BTreeSet<_>>();

        if !names.contains("config.json") {
            return Err(ModelError::NotFound(
                "repository is missing config.json".to_owned(),
            ));
        }
        if !names.contains("tokenizer.json") {
            return Err(ModelError::NotFound(
                "repository is missing tokenizer.json".to_owned(),
            ));
        }

        let pinned = api.repo(Repo::with_revision(
            reference.repository.clone(),
            RepoType::Model,
            revision.clone(),
        ));

        let config_path = pinned
            .get("config.json")
            .await
            .map_err(|error| ModelError::Provider(error.to_string()))?;
        let config_bytes = tokio::fs::read(&config_path).await?;
        let config: serde_json::Value = serde_json::from_slice(&config_bytes)?;

        let tokenizer_path = pinned
            .get("tokenizer.json")
            .await
            .map_err(|error| ModelError::Provider(error.to_string()))?;
        let tokenizer_bytes = tokio::fs::read(&tokenizer_path).await?;
        let tokenizer_hash = hash_bytes_hex(&tokenizer_bytes);

        let mut tokenizer_artifacts = vec!["tokenizer.json".to_owned()];
        for candidate in [
            "tokenizer_config.json",
            "special_tokens_map.json",
            "vocab.json",
            "merges.txt",
            "chat_template.jinja",
            "chat_template.json",
        ] {
            if names.contains(candidate) {
                tokenizer_artifacts.push(candidate.to_owned());
            }
        }

        let weight_files = resolve_weight_files(&names, &pinned).await?;
        let mut shards = Vec::with_capacity(weight_files.len());
        let mut artifact_refs = Vec::new();

        for relative_path in weight_files {
            let meta = self
                .read_metadata_inner(&reference.repository, &revision, &relative_path)
                .await?;
            let header = self
                .discover_header(&reference.repository, &revision, &relative_path, &meta)
                .await?;
            artifact_refs.push(ArtifactRef {
                provider: PROVIDER_HUGGINGFACE.to_owned(),
                repository: reference.repository.clone(),
                revision: revision.clone(),
                relative_path: relative_path.clone(),
                size_bytes: meta.size_bytes,
                etag: meta.etag.clone(),
                digest_hex: meta.digest_hex.clone(),
            });
            shards.push(WeightShard {
                relative_path,
                size_bytes: meta.size_bytes,
                etag: meta.etag,
                digest_hex: meta.digest_hex,
                header,
            });
        }

        let mut extra_artifacts = vec![
            ArtifactRecord {
                relative_path: "config.json".to_owned(),
                size_bytes: Some(config_bytes.len() as u64),
                etag: None,
                digest_hex: Some(hash_bytes_hex(&config_bytes)),
            },
            ArtifactRecord {
                relative_path: "tokenizer.json".to_owned(),
                size_bytes: Some(tokenizer_bytes.len() as u64),
                etag: None,
                digest_hex: Some(tokenizer_hash.clone()),
            },
        ];
        if names.contains("model.safetensors.index.json") {
            extra_artifacts.push(ArtifactRecord {
                relative_path: "model.safetensors.index.json".to_owned(),
                size_bytes: None,
                etag: None,
                digest_hex: None,
            });
        }

        let manifest = build_qwen3_dense_manifest(AdapterInputs {
            repository: reference.repository.clone(),
            revision: revision.clone(),
            config,
            tokenizer_artifacts,
            tokenizer_hash,
            shards,
            extra_artifacts,
        })?;
        let identity = build_manifest_identity(&manifest)?;
        let mut artifacts = artifact_refs;
        for record in &manifest.artifacts {
            if !artifacts
                .iter()
                .any(|item| item.relative_path == record.relative_path)
            {
                artifacts.push(ArtifactRef {
                    provider: PROVIDER_HUGGINGFACE.to_owned(),
                    repository: reference.repository.clone(),
                    revision: revision.clone(),
                    relative_path: record.relative_path.clone(),
                    size_bytes: record.size_bytes,
                    etag: record.etag.clone(),
                    digest_hex: record.digest_hex.clone(),
                });
            }
        }
        let local_artifacts = BTreeMap::from([
            ("config.json".to_owned(), config_path),
            ("tokenizer.json".to_owned(), tokenizer_path),
        ]);

        Ok(ResolvedModel {
            identity,
            manifest,
            artifacts,
            local_artifacts,
        })
    }

    pub async fn read_metadata(&self, artifact: &ArtifactRef) -> ModelResult<ArtifactMetadata> {
        self.read_metadata_inner(
            &artifact.repository,
            &artifact.revision,
            &artifact.relative_path,
        )
        .await
    }

    async fn read_metadata_inner(
        &self,
        repository: &str,
        revision: &str,
        relative_path: &str,
    ) -> ModelResult<ArtifactMetadata> {
        let url = self.resolve_url(repository, revision, relative_path);
        let request = self.authorize(self.meta_client.head(&url));
        let response = request.send().await?;
        if response.status() == StatusCode::NOT_FOUND {
            return Err(ModelError::NotFound(relative_path.to_owned()));
        }
        if response.status() == StatusCode::UNAUTHORIZED
            || response.status() == StatusCode::FORBIDDEN
        {
            return Err(ModelError::Access(format!(
                "metadata denied for {relative_path}: {}",
                response.status()
            )));
        }
        if response.status().is_redirection()
            || response.status().is_success()
            || response.status() == StatusCode::PARTIAL_CONTENT
            || response.status() == StatusCode::RANGE_NOT_SATISFIABLE
        {
            if let Some(meta) = metadata_from_headers(relative_path, response.headers()) {
                return Ok(meta);
            }
        }
        // Some endpoints dislike HEAD; fall back to a one-byte range without following redirects.
        self.metadata_via_range(repository, revision, relative_path)
            .await
    }

    async fn metadata_via_range(
        &self,
        repository: &str,
        revision: &str,
        relative_path: &str,
    ) -> ModelResult<ArtifactMetadata> {
        let url = self.resolve_url(repository, revision, relative_path);
        let request = self
            .authorize(self.meta_client.get(&url))
            .header(RANGE, "bytes=0-0");
        let response = request.send().await?;
        if response.status() == StatusCode::UNAUTHORIZED
            || response.status() == StatusCode::FORBIDDEN
        {
            return Err(ModelError::Access(format!(
                "metadata denied for {relative_path}"
            )));
        }
        if !(response.status().is_redirection()
            || response.status() == StatusCode::PARTIAL_CONTENT
            || response.status().is_success())
        {
            return Err(ModelError::Http(format!(
                "metadata range failed for {relative_path}: {}",
                response.status()
            )));
        }
        metadata_from_headers(relative_path, response.headers()).ok_or_else(|| {
            ModelError::Invalid(format!("missing size/etag metadata for {relative_path}"))
        })
    }

    async fn discover_header(
        &self,
        repository: &str,
        revision: &str,
        relative_path: &str,
        meta: &ArtifactMetadata,
    ) -> ModelResult<crate::safetensors::SafetensorsHeader> {
        match self
            .fetch_range_inner(repository, revision, relative_path, 0..8)
            .await
        {
            Ok(prefix) => {
                let header_len = parse_header_length(&prefix)?;
                if 8 + header_len > MAX_HEADER_FETCH {
                    return Err(ModelError::Invalid(format!(
                        "safetensors header too large on {relative_path}"
                    )));
                }
                let header_bytes = self
                    .fetch_range_inner(repository, revision, relative_path, 8..(8 + header_len))
                    .await?;
                parse_safetensors_header(header_len, &header_bytes)
            }
            Err(ModelError::Unsupported(_)) | Err(ModelError::Http(_)) => {
                let tmp = self.hf_cache_dir.join(format!(
                    "header-fallback-{}-{}.safetensors",
                    revision,
                    relative_path.replace('/', "_")
                ));
                let mut ignore_progress = |_| {};
                self.fetch_file_inner(
                    repository,
                    revision,
                    relative_path,
                    &tmp,
                    &mut ignore_progress,
                )
                .await?;
                let file = tokio::fs::read(&tmp).await?;
                let _ = tokio::fs::remove_file(&tmp).await;
                if file.len() < 8 {
                    return Err(ModelError::Invalid(
                        "weight artifact too small for safetensors header".to_owned(),
                    ));
                }
                let header_len = parse_header_length(&file[..8])?;
                let end = 8 + header_len as usize;
                if file.len() < end {
                    return Err(ModelError::Invalid(
                        "weight artifact truncated before safetensors header end".to_owned(),
                    ));
                }
                let _ = meta;
                parse_safetensors_header(header_len, &file[8..end])
            }
            Err(error) => Err(error),
        }
    }

    pub async fn fetch_range(
        &self,
        artifact: &ArtifactRef,
        range: Range<u64>,
    ) -> ModelResult<Bytes> {
        self.fetch_range_inner(
            &artifact.repository,
            &artifact.revision,
            &artifact.relative_path,
            range,
        )
        .await
    }

    async fn fetch_range_inner(
        &self,
        repository: &str,
        revision: &str,
        relative_path: &str,
        range: Range<u64>,
    ) -> ModelResult<Bytes> {
        if range.end <= range.start {
            return Err(ModelError::Invalid(
                "range end must be greater than start".to_owned(),
            ));
        }
        let end_inclusive = range.end - 1;
        let url = self.resolve_url(repository, revision, relative_path);
        let request = self
            .authorize(self.client.get(&url))
            .header(RANGE, format!("bytes={}-{}", range.start, end_inclusive));
        let response = request.send().await?;
        let status = response.status();
        if status == StatusCode::OK {
            return Err(ModelError::Unsupported(format!(
                "range unsupported for {relative_path}"
            )));
        }
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(ModelError::Access(format!(
                "range denied for {relative_path}"
            )));
        }
        if status == StatusCode::NOT_FOUND {
            return Err(ModelError::NotFound(relative_path.to_owned()));
        }
        if status != StatusCode::PARTIAL_CONTENT {
            return Err(ModelError::Http(format!(
                "unexpected status {status} for range on {relative_path}"
            )));
        }
        let content_range = response
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| {
                ModelError::Invalid(format!("missing Content-Range for {relative_path}"))
            })?;
        let parsed = parse_content_range_header(content_range)?;
        let body = response.bytes().await?;
        validate_content_range(&RangeValidation {
            requested_start: range.start,
            requested_end_exclusive: range.end,
            body_len: body.len() as u64,
            content_range: parsed,
            expected_total: None,
        })?;
        Ok(body)
    }

    pub async fn fetch_file(
        &self,
        artifact: &ArtifactRef,
        destination: &Path,
        on_progress: &mut (dyn FnMut(u64) + Send),
    ) -> ModelResult<()> {
        self.fetch_file_inner(
            &artifact.repository,
            &artifact.revision,
            &artifact.relative_path,
            destination,
            on_progress,
        )
        .await
    }

    async fn fetch_file_inner(
        &self,
        repository: &str,
        revision: &str,
        relative_path: &str,
        destination: &Path,
        on_progress: &mut (dyn FnMut(u64) + Send),
    ) -> ModelResult<()> {
        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let url = self.resolve_url(repository, revision, relative_path);
        let request = self.authorize(self.client.get(&url));
        let response = request.send().await?;
        let status = response.status();
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN {
            return Err(ModelError::Access(format!(
                "download denied for {relative_path}"
            )));
        }
        if status == StatusCode::NOT_FOUND {
            return Err(ModelError::NotFound(relative_path.to_owned()));
        }
        if !status.is_success() {
            return Err(ModelError::Http(format!(
                "download failed for {relative_path}: {status}"
            )));
        }
        let mut stream = response.bytes_stream();
        let mut file = tokio::fs::File::create(destination).await?;
        let mut bytes_written = 0u64;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            file.write_all(&chunk).await?;
            bytes_written = bytes_written.saturating_add(chunk.len() as u64);
            on_progress(bytes_written);
        }
        file.flush().await?;
        Ok(())
    }
}

async fn resolve_weight_files(
    names: &BTreeSet<String>,
    pinned: &hf_hub::api::tokio::ApiRepo,
) -> ModelResult<Vec<String>> {
    if names.contains("model.safetensors.index.json") {
        let index_path = pinned
            .get("model.safetensors.index.json")
            .await
            .map_err(|error| ModelError::Provider(error.to_string()))?;
        let bytes = tokio::fs::read(index_path).await?;
        let index: SafetensorsIndex = serde_json::from_slice(&bytes)?;
        let mut files = index
            .weight_map
            .values()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        if files.is_empty() {
            return Err(ModelError::Invalid(
                "safetensors index weight_map is empty".to_owned(),
            ));
        }
        files.sort();
        return Ok(files);
    }
    if names.contains("model.safetensors") {
        return Ok(vec!["model.safetensors".to_owned()]);
    }
    let mut single = names
        .iter()
        .filter(|name| name.ends_with(".safetensors") && !name.contains('/'))
        .cloned()
        .collect::<Vec<_>>();
    single.sort();
    if single.len() == 1 {
        return Ok(single);
    }
    Err(ModelError::NotFound(
        "repository has no model.safetensors or sharded index".to_owned(),
    ))
}

#[derive(Debug, Deserialize)]
struct SafetensorsIndex {
    weight_map: BTreeMap<String, String>,
}

fn header_str<'a>(headers: &'a reqwest::header::HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn metadata_from_headers(
    relative_path: &str,
    headers: &reqwest::header::HeaderMap,
) -> Option<ArtifactMetadata> {
    let linked_etag =
        header_str(headers, "x-linked-etag").map(|value| value.trim_matches('"').to_owned());
    let plain_etag = headers
        .get(ETAG)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim_matches('"').to_owned());
    let etag = linked_etag.clone().or(plain_etag);
    let size_bytes = header_str(headers, "x-linked-size")
        .and_then(|value| value.parse().ok())
        .or_else(|| {
            headers
                .get(CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse().ok())
        })
        .or_else(|| {
            header_str(headers, CONTENT_RANGE.as_str())
                .and_then(|value| parse_content_range_header(value).ok())
                .and_then(|range| range.total)
        });
    let commit_sha = header_str(headers, "x-repo-commit").map(|value| value.to_ascii_lowercase());
    let digest_hex = linked_etag
        .as_deref()
        .and_then(etag_to_digest)
        .or_else(|| etag.as_deref().and_then(etag_to_digest));

    if size_bytes.is_none() && etag.is_none() && digest_hex.is_none() {
        return None;
    }

    Some(ArtifactMetadata {
        relative_path: relative_path.to_owned(),
        size_bytes,
        etag,
        digest_hex,
        commit_sha,
    })
}

fn etag_to_digest(etag: &str) -> Option<String> {
    let cleaned = etag.trim().trim_matches('"');
    if cleaned.len() == 64 && cleaned.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Some(cleaned.to_ascii_lowercase())
    } else if let Some(hash) = cleaned.strip_prefix("sha256:") {
        if hash.len() == 64 {
            Some(hash.to_ascii_lowercase())
        } else {
            None
        }
    } else {
        None
    }
}

fn classify_provider_error(text: String) -> ModelError {
    let lower = text.to_ascii_lowercase();
    if lower.contains("401") || lower.contains("403") || lower.contains("unauthorized") {
        ModelError::Access(text)
    } else if lower.contains("404") || lower.contains("not found") {
        ModelError::NotFound(text)
    } else {
        ModelError::Provider(text)
    }
}

pub fn identity_summary(identity: &ModelIdentity) -> String {
    format!(
        "{}@{} fmt={} hash={}",
        identity.repository,
        &identity.revision[..identity.revision.len().min(12)],
        identity.model_format.as_str(),
        &identity.manifest_hash[..identity.manifest_hash.len().min(12)]
    )
}

pub fn sha256_file(path: &Path) -> ModelResult<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn etag_digest_parsing() {
        assert_eq!(
            etag_to_digest("\"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\""),
            Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_owned())
        );
        assert_eq!(etag_to_digest("W/\"abc\""), None);
    }

    #[test]
    fn prefers_linked_etag_digest_over_cdn_etag() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(
            ETAG,
            "\"a6f5dec111c34cd267ff4fd7889ef961237b30418d123d5b60b2c1fd3cbd3cc7\""
                .parse()
                .unwrap(),
        );
        headers.insert(
            "x-linked-etag",
            "\"328a91d3122359d5547f9d79521205bc0a46e1f79a792dfe650e99fc2d651223\""
                .parse()
                .unwrap(),
        );
        headers.insert("x-linked-size", "3957900840".parse().unwrap());
        let meta = metadata_from_headers("model-00001-of-00003.safetensors", &headers).unwrap();
        assert_eq!(
            meta.digest_hex.as_deref(),
            Some("328a91d3122359d5547f9d79521205bc0a46e1f79a792dfe650e99fc2d651223")
        );
        assert_eq!(meta.size_bytes, Some(3957900840));
    }
}
