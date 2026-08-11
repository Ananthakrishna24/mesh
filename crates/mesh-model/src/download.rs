use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use mesh_core::{
    CacheValidationState, ModelCacheEntry, ModelDownloadProgress, ModelIdentity, now_unix_ms,
    prepare_disk_margin,
};
use sha2::{Digest, Sha256};

use crate::cache::{
    absolute_cache_path, artifact_path_hash, complete_entry_id, complete_object_rel_path,
    ensure_parent, file_len, partial_meta_path, partial_path, publish_file, range_entry_id,
    range_object_rel_path, repository_hash, validate_entry_file,
};
use crate::huggingface::HuggingFaceProvider;
use crate::manifest::{CanonicalManifest, TensorRecord};
use crate::provider::{
    ArtifactRef, NodeModelPlan, PreparedArtifact, ResolvedModel, TensorAssignment,
};
use crate::safetensors::{
    SafetensorsDtype, default_merge_ranges, dtype_width_bytes, parse_header_length,
    parse_safetensors_header, tensor_payload_absolute_range,
};
use crate::validate::validate_tensor_byte_length;
use crate::{ModelError, ModelResult};
use bytes::Bytes;
use std::ops::Range;
use std::pin::Pin;

const COMPLETE_SHARD_COVERAGE_NUM: u64 = 80;
const COMPLETE_SHARD_COVERAGE_DEN: u64 = 100;
const MAX_RANGE_ATTEMPTS: u32 = 3;
const PROGRESS_REPORT_INTERVAL_BYTES: u64 = 8 * 1024 * 1024;

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait FetchSource: Send + Sync {
    fn fetch_file<'a>(
        &'a self,
        artifact: &'a ArtifactRef,
        destination: &'a Path,
        on_progress: &'a mut (dyn FnMut(u64) + Send),
    ) -> BoxFuture<'a, ModelResult<()>>;

    fn fetch_range<'a>(
        &'a self,
        artifact: &'a ArtifactRef,
        range: Range<u64>,
    ) -> BoxFuture<'a, ModelResult<Bytes>>;
}
impl FetchSource for HuggingFaceProvider {
    fn fetch_file<'a>(
        &'a self,
        artifact: &'a ArtifactRef,
        destination: &'a Path,
        on_progress: &'a mut (dyn FnMut(u64) + Send),
    ) -> BoxFuture<'a, ModelResult<()>> {
        Box::pin(async move {
            HuggingFaceProvider::fetch_file(self, artifact, destination, on_progress).await
        })
    }

    fn fetch_range<'a>(
        &'a self,
        artifact: &'a ArtifactRef,
        range: Range<u64>,
    ) -> BoxFuture<'a, ModelResult<Bytes>> {
        Box::pin(async move { HuggingFaceProvider::fetch_range(self, artifact, range).await })
    }
}

#[derive(Debug, Clone)]
pub struct DownloadProgressEvent {
    pub progress: ModelDownloadProgress,
}

pub trait ProgressSink: Send {
    fn on_progress(&mut self, event: DownloadProgressEvent);
}

pub struct NoopProgress;

impl ProgressSink for NoopProgress {
    fn on_progress(&mut self, _event: DownloadProgressEvent) {}
}

#[derive(Debug, Clone)]
pub struct PrepareResult {
    pub identity: ModelIdentity,
    pub plan: NodeModelPlan,
    pub prepared: Vec<PreparedArtifact>,
    pub cache_entries: Vec<ModelCacheEntry>,
    pub bytes_downloaded: u64,
    pub bytes_from_cache: u64,
    pub summary: String,
}

pub fn build_complete_plan(
    deployment_id: impl Into<String>,
    resolved: &ResolvedModel,
) -> ModelResult<NodeModelPlan> {
    let deployment_id = deployment_id.into();
    let manifest = &resolved.manifest;
    let layer_indices = manifest
        .tensors
        .iter()
        .filter_map(|tensor| tensor.layer_index)
        .collect::<BTreeSet<_>>();
    let first_layer = layer_indices.iter().copied().min().unwrap_or(0);
    let last_layer_exclusive = layer_indices
        .iter()
        .copied()
        .max()
        .map(|value| value.saturating_add(1))
        .unwrap_or(0);

    let mut tensor_assignments = Vec::new();
    let mut global_tensors = Vec::new();
    for tensor in &manifest.tensors {
        let assignment = TensorAssignment {
            name: tensor.name.clone(),
            dtype: tensor.dtype.clone(),
            shape: tensor.shape.clone(),
            artifact_path: tensor.artifact_path.clone(),
            absolute_start: tensor.absolute_start,
            absolute_end: tensor.absolute_end,
            layer_index: tensor.layer_index,
        };
        if tensor.layer_index.is_some() {
            tensor_assignments.push(assignment);
        } else {
            global_tensors.push(assignment);
        }
    }
    tensor_assignments.sort_by(|left, right| left.name.cmp(&right.name));
    global_tensors.sort_by(|left, right| left.name.cmp(&right.name));

    let disk_bytes_required = estimate_disk_bytes(manifest, &tensor_assignments, &global_tensors);
    let assignment_hash = assignment_hash(
        &deployment_id,
        &resolved.identity,
        first_layer,
        last_layer_exclusive,
        &tensor_assignments,
        &global_tensors,
    );

    Ok(NodeModelPlan {
        deployment_id,
        model: resolved.identity.clone(),
        assignment_hash,
        first_layer,
        last_layer_exclusive,
        tensor_assignments,
        global_tensors,
        disk_bytes_required,
        gpu_bytes_reserved: manifest.memory_estimate_bytes,
    })
}

pub fn build_layer_plan(
    deployment_id: impl Into<String>,
    resolved: &ResolvedModel,
    first_layer: u32,
    last_layer_exclusive: u32,
    include_embeddings: bool,
    include_final: bool,
) -> ModelResult<NodeModelPlan> {
    if last_layer_exclusive < first_layer {
        return Err(ModelError::Invalid(
            "last_layer_exclusive must be >= first_layer".to_owned(),
        ));
    }
    let manifest = &resolved.manifest;
    let mut tensor_assignments = Vec::new();
    let mut global_tensors = Vec::new();
    for tensor in &manifest.tensors {
        match tensor.layer_index {
            Some(index) if index >= first_layer && index < last_layer_exclusive => {
                tensor_assignments.push(to_assignment(tensor));
            }
            Some(_) => {}
            None => {
                let include = match tensor.role {
                    crate::manifest::TensorRole::Embedding => include_embeddings,
                    crate::manifest::TensorRole::FinalNorm
                    | crate::manifest::TensorRole::LmHead => include_final,
                    crate::manifest::TensorRole::Layer => false,
                    crate::manifest::TensorRole::Other => include_embeddings || include_final,
                };
                if include {
                    global_tensors.push(to_assignment(tensor));
                }
            }
        }
    }
    tensor_assignments.sort_by(|left, right| left.name.cmp(&right.name));
    global_tensors.sort_by(|left, right| left.name.cmp(&right.name));
    let disk_bytes_required = estimate_disk_bytes(manifest, &tensor_assignments, &global_tensors);
    let deployment_id = deployment_id.into();
    let assignment_hash = assignment_hash(
        &deployment_id,
        &resolved.identity,
        first_layer,
        last_layer_exclusive,
        &tensor_assignments,
        &global_tensors,
    );
    Ok(NodeModelPlan {
        deployment_id,
        model: resolved.identity.clone(),
        assignment_hash,
        first_layer,
        last_layer_exclusive,
        tensor_assignments,
        global_tensors,
        disk_bytes_required,
        gpu_bytes_reserved: disk_bytes_required,
    })
}

pub fn stage_plan_flags(role: mesh_core::StageRole, tie_word_embeddings: bool) -> (bool, bool) {
    let include_embeddings =
        role.owns_embeddings() || (role.owns_output_head() && tie_word_embeddings);
    let include_final = role.owns_output_head();
    (include_embeddings, include_final)
}

pub fn resolved_tie_word_embeddings(resolved: &ResolvedModel) -> bool {
    resolved
        .manifest
        .architecture
        .get("tie_word_embeddings")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

pub fn build_stage_plan(
    deployment_id: impl Into<String>,
    resolved: &ResolvedModel,
    role: mesh_core::StageRole,
    layer_range: mesh_core::LayerRange,
) -> ModelResult<NodeModelPlan> {
    let tie = resolved_tie_word_embeddings(resolved);
    let (include_embeddings, include_final) = stage_plan_flags(role, tie);
    build_layer_plan(
        deployment_id,
        resolved,
        layer_range.start,
        layer_range.end,
        include_embeddings,
        include_final,
    )
}

#[derive(Clone)]
struct OwnedTensorView {
    dtype: safetensors::Dtype,
    shape: Vec<usize>,
    data: Vec<u8>,
}

impl safetensors::View for OwnedTensorView {
    fn dtype(&self) -> safetensors::Dtype {
        self.dtype
    }

    fn shape(&self) -> &[usize] {
        &self.shape
    }

    fn data(&self) -> std::borrow::Cow<'_, [u8]> {
        std::borrow::Cow::Borrowed(&self.data)
    }

    fn data_len(&self) -> usize {
        self.data.len()
    }
}

fn mesh_dtype_to_safetensors(dtype: SafetensorsDtype) -> ModelResult<safetensors::Dtype> {
    Ok(match dtype {
        SafetensorsDtype::Bool => safetensors::Dtype::BOOL,
        SafetensorsDtype::U8 => safetensors::Dtype::U8,
        SafetensorsDtype::I8 => safetensors::Dtype::I8,
        SafetensorsDtype::I16 => safetensors::Dtype::I16,
        SafetensorsDtype::U16 => safetensors::Dtype::U16,
        SafetensorsDtype::F16 => safetensors::Dtype::F16,
        SafetensorsDtype::Bf16 => safetensors::Dtype::BF16,
        SafetensorsDtype::I32 => safetensors::Dtype::I32,
        SafetensorsDtype::U32 => safetensors::Dtype::U32,
        SafetensorsDtype::F32 => safetensors::Dtype::F32,
        SafetensorsDtype::I64 => safetensors::Dtype::I64,
        SafetensorsDtype::U64 => safetensors::Dtype::U64,
        SafetensorsDtype::F64 => safetensors::Dtype::F64,
    })
}

fn stage_weight_rel_path(
    identity: &mesh_core::ModelIdentity,
    assignment_hash: &str,
    artifact_path: &str,
) -> String {
    format!(
        "stages/{}/{}/{}/{}/{}",
        identity.provider,
        repository_hash(&identity.repository),
        identity.revision,
        &assignment_hash[..assignment_hash.len().min(16)],
        artifact_path_hash(artifact_path)
    )
}

fn find_prepared_cover<'a>(
    prepared: &'a [PreparedArtifact],
    assignment: &TensorAssignment,
) -> Option<&'a PreparedArtifact> {
    prepared.iter().find(|entry| {
        if entry.artifact_path != assignment.artifact_path {
            return false;
        }
        match (entry.range_start, entry.range_end) {
            (None, None) => true,
            (Some(start), Some(end)) => {
                start <= assignment.absolute_start && end >= assignment.absolute_end
            }
            _ => false,
        }
    })
}

fn read_assignment_bytes(
    cache_root: &Path,
    cover: &PreparedArtifact,
    assignment: &TensorAssignment,
) -> ModelResult<Vec<u8>> {
    let path = cache_root.join(&cover.relative_path);
    let file = std::fs::File::open(&path)?;
    let mut file = std::io::BufReader::new(file);
    use std::io::{Read, Seek, SeekFrom};
    let (offset, len) = match (cover.range_start, cover.range_end) {
        (None, None) => {
            let start = assignment.absolute_start;
            let end = assignment.absolute_end;
            if end < start {
                return Err(ModelError::Invalid(format!(
                    "invalid tensor range for {}",
                    assignment.name
                )));
            }
            (start, end - start)
        }
        (Some(range_start), Some(range_end)) => {
            if assignment.absolute_start < range_start || assignment.absolute_end > range_end {
                return Err(ModelError::Invalid(format!(
                    "prepared range does not cover {}",
                    assignment.name
                )));
            }
            (
                assignment.absolute_start - range_start,
                assignment.absolute_end - assignment.absolute_start,
            )
        }
        _ => {
            return Err(ModelError::Invalid(format!(
                "incomplete range metadata for {}",
                assignment.name
            )));
        }
    };
    file.seek(SeekFrom::Start(offset))?;
    let mut buf = vec![0u8; len as usize];
    file.read_exact(&mut buf)?;
    Ok(buf)
}

/// Build mmap-able safetensors files that contain only the tensors in `prepared.plan`.
/// Whole-shard cache hits are reused as-is; pure range artifacts are rewritten under `stages/`.
pub fn materialize_stage_weight_files(
    cache_root: &Path,
    prepared: &PrepareResult,
) -> ModelResult<Vec<(String, PathBuf)>> {
    let mut by_artifact: BTreeMap<String, Vec<&TensorAssignment>> = BTreeMap::new();
    for assignment in prepared
        .plan
        .tensor_assignments
        .iter()
        .chain(prepared.plan.global_tensors.iter())
    {
        by_artifact
            .entry(assignment.artifact_path.clone())
            .or_default()
            .push(assignment);
    }
    if by_artifact.is_empty() {
        return Err(ModelError::Invalid(
            "stage prepare plan has no tensors to materialize".to_owned(),
        ));
    }

    let mut out = Vec::with_capacity(by_artifact.len());
    for (artifact_path, assignments) in by_artifact {
        let covers = assignments
            .iter()
            .map(|assignment| {
                find_prepared_cover(&prepared.prepared, assignment).ok_or_else(|| {
                    ModelError::Invalid(format!(
                        "missing prepared cover for tensor {} in {}",
                        assignment.name, artifact_path
                    ))
                })
            })
            .collect::<ModelResult<Vec<_>>>()?;

        let whole = covers.iter().find(|cover| {
            cover.range_start.is_none()
                && cover.range_end.is_none()
                && cache_root.join(&cover.relative_path).is_file()
        });
        if let Some(cover) = whole {
            out.push((artifact_path, cache_root.join(&cover.relative_path)));
            continue;
        }

        let mut tensors = Vec::with_capacity(assignments.len());
        for (assignment, cover) in assignments.iter().zip(covers.iter()) {
            let dtype = SafetensorsDtype::parse(&assignment.dtype).ok_or_else(|| {
                ModelError::Unsupported(format!("dtype {} unsupported", assignment.dtype))
            })?;
            validate_tensor_byte_length(
                dtype,
                &assignment.shape,
                assignment.absolute_start,
                assignment.absolute_end,
            )?;
            let data = read_assignment_bytes(cache_root, cover, assignment)?;
            let shape = assignment
                .shape
                .iter()
                .map(|dim| *dim as usize)
                .collect::<Vec<_>>();
            tensors.push((
                assignment.name.clone(),
                OwnedTensorView {
                    dtype: mesh_dtype_to_safetensors(dtype)?,
                    shape,
                    data,
                },
            ));
        }

        let relative = stage_weight_rel_path(
            &prepared.identity,
            &prepared.plan.assignment_hash,
            &artifact_path,
        );
        let absolute = absolute_cache_path(cache_root, &relative);
        if !absolute.is_file() {
            ensure_parent(&absolute)?;
            let bytes = safetensors::serialize(tensors, &None).map_err(|error| {
                ModelError::Invalid(format!("stage safetensors serialize failed: {error}"))
            })?;
            let partial = partial_path(&absolute);
            std::fs::write(&partial, bytes)?;
            publish_file(&partial, &absolute)?;
        }
        out.push((artifact_path, absolute));
    }

    out.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(out)
}

fn to_assignment(tensor: &TensorRecord) -> TensorAssignment {
    TensorAssignment {
        name: tensor.name.clone(),
        dtype: tensor.dtype.clone(),
        shape: tensor.shape.clone(),
        artifact_path: tensor.artifact_path.clone(),
        absolute_start: tensor.absolute_start,
        absolute_end: tensor.absolute_end,
        layer_index: tensor.layer_index,
    }
}

fn estimate_disk_bytes(
    manifest: &CanonicalManifest,
    layers: &[TensorAssignment],
    globals: &[TensorAssignment],
) -> u64 {
    let mut by_artifact: BTreeMap<String, Vec<(u64, u64)>> = BTreeMap::new();
    for tensor in layers.iter().chain(globals.iter()) {
        by_artifact
            .entry(tensor.artifact_path.clone())
            .or_default()
            .push((tensor.absolute_start, tensor.absolute_end));
    }
    let mut total = 0u64;
    for (path, ranges) in by_artifact {
        let merged = default_merge_ranges(&ranges);
        let covered = merged
            .iter()
            .map(|(start, end)| end.saturating_sub(*start))
            .sum::<u64>();
        let artifact_size = manifest
            .artifacts
            .iter()
            .find(|item| item.relative_path == path)
            .and_then(|item| item.size_bytes)
            .unwrap_or(covered);
        if artifact_size > 0
            && covered.saturating_mul(COMPLETE_SHARD_COVERAGE_DEN)
                >= artifact_size.saturating_mul(COMPLETE_SHARD_COVERAGE_NUM)
        {
            total = total.saturating_add(artifact_size);
        } else {
            total = total.saturating_add(covered);
        }
    }
    prepare_disk_margin(total)
}

fn assignment_hash(
    deployment_id: &str,
    identity: &ModelIdentity,
    first_layer: u32,
    last_layer_exclusive: u32,
    layers: &[TensorAssignment],
    globals: &[TensorAssignment],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(deployment_id.as_bytes());
    hasher.update(identity.provider.as_bytes());
    hasher.update(identity.repository.as_bytes());
    hasher.update(identity.revision.as_bytes());
    hasher.update(identity.manifest_hash.as_bytes());
    hasher.update(first_layer.to_le_bytes());
    hasher.update(last_layer_exclusive.to_le_bytes());
    for tensor in layers.iter().chain(globals.iter()) {
        hasher.update(tensor.name.as_bytes());
        hasher.update(tensor.artifact_path.as_bytes());
        hasher.update(tensor.absolute_start.to_le_bytes());
        hasher.update(tensor.absolute_end.to_le_bytes());
    }
    hex::encode(hasher.finalize())
}

pub fn net_disk_bytes_required(
    plan: &NodeModelPlan,
    root: &Path,
    entries: &[ModelCacheEntry],
) -> u64 {
    let mut remaining = plan.disk_bytes_required;
    for assignment in plan
        .tensor_assignments
        .iter()
        .chain(plan.global_tensors.iter())
    {
        if let Some(entry) = find_covering_entry(entries, &plan.model, assignment) {
            if validate_entry_file(root, entry).unwrap_or(false) {
                let covered = assignment
                    .absolute_end
                    .saturating_sub(assignment.absolute_start);
                remaining = remaining.saturating_sub(covered);
            }
        }
    }
    remaining
}

fn find_covering_entry<'a>(
    entries: &'a [ModelCacheEntry],
    identity: &ModelIdentity,
    assignment: &TensorAssignment,
) -> Option<&'a ModelCacheEntry> {
    entries.iter().find(|entry| {
        entry.state == CacheValidationState::Valid
            && entry.provider == identity.provider
            && entry.repository == identity.repository
            && entry.revision == identity.revision
            && entry.artifact_path == assignment.artifact_path
            && match (entry.range_start, entry.range_end) {
                (None, None) => true,
                (Some(start), Some(end)) => {
                    start <= assignment.absolute_start && end >= assignment.absolute_end
                }
                _ => false,
            }
    })
}

pub async fn prepare_plan(
    provider: &dyn FetchSource,
    resolved: &ResolvedModel,
    plan: &NodeModelPlan,
    cache_root: &Path,
    existing: &[ModelCacheEntry],
    progress: &mut dyn ProgressSink,
) -> ModelResult<PrepareResult> {
    let mut cache_entries = Vec::new();
    let mut prepared = Vec::new();
    let mut bytes_downloaded = 0u64;
    let mut bytes_from_cache = 0u64;

    let mut needed_by_artifact: BTreeMap<String, Vec<TensorRecord>> = BTreeMap::new();
    for assignment in plan
        .tensor_assignments
        .iter()
        .chain(plan.global_tensors.iter())
    {
        if let Some(entry) = find_covering_entry(existing, &resolved.identity, assignment) {
            if validate_entry_file(cache_root, entry).unwrap_or(false) {
                bytes_from_cache = bytes_from_cache.saturating_add(
                    assignment
                        .absolute_end
                        .saturating_sub(assignment.absolute_start),
                );
                prepared.push(PreparedArtifact {
                    entry_id: entry.entry_id.clone(),
                    relative_path: PathBuf::from(&entry.relative_path),
                    artifact_path: entry.artifact_path.clone(),
                    byte_length: entry.byte_length,
                    range_start: entry.range_start,
                    range_end: entry.range_end,
                });
                continue;
            }
        }
        let tensor = resolved
            .manifest
            .tensors
            .iter()
            .find(|item| item.name == assignment.name)
            .cloned()
            .ok_or_else(|| ModelError::Invalid(format!("missing tensor {}", assignment.name)))?;
        needed_by_artifact
            .entry(assignment.artifact_path.clone())
            .or_default()
            .push(tensor);
    }

    for (artifact_path, tensors) in needed_by_artifact {
        let artifact = resolved
            .artifacts
            .iter()
            .find(|item| item.relative_path == artifact_path)
            .cloned()
            .unwrap_or(ArtifactRef {
                provider: resolved.identity.provider.clone(),
                repository: resolved.identity.repository.clone(),
                revision: resolved.identity.revision.clone(),
                relative_path: artifact_path.clone(),
                size_bytes: resolved
                    .manifest
                    .artifacts
                    .iter()
                    .find(|item| item.relative_path == artifact_path)
                    .and_then(|item| item.size_bytes),
                etag: None,
                digest_hex: None,
            });

        let ranges = tensors
            .iter()
            .map(|tensor| (tensor.absolute_start, tensor.absolute_end))
            .collect::<Vec<_>>();
        let merged = default_merge_ranges(&ranges);
        let covered = merged
            .iter()
            .map(|(start, end)| end.saturating_sub(*start))
            .sum::<u64>();
        let artifact_size = artifact.size_bytes.unwrap_or(covered);
        let prefer_complete = artifact_size > 0
            && covered.saturating_mul(COMPLETE_SHARD_COVERAGE_DEN)
                >= artifact_size.saturating_mul(COMPLETE_SHARD_COVERAGE_NUM);

        let outcome = if prefer_complete {
            download_complete_artifact(
                provider,
                &artifact,
                cache_root,
                &resolved.identity,
                progress,
            )
            .await
        } else {
            match download_ranges(
                provider,
                &artifact,
                &merged,
                &tensors,
                cache_root,
                &resolved.identity,
                progress,
            )
            .await
            {
                Ok(value) => Ok(value),
                Err(ModelError::Unsupported(_)) => {
                    download_complete_artifact(
                        provider,
                        &artifact,
                        cache_root,
                        &resolved.identity,
                        progress,
                    )
                    .await
                }
                Err(error) => Err(error),
            }
        }?;

        bytes_downloaded = bytes_downloaded.saturating_add(outcome.bytes_downloaded);
        for entry in outcome.entries {
            prepared.push(PreparedArtifact {
                entry_id: entry.entry_id.clone(),
                relative_path: PathBuf::from(&entry.relative_path),
                artifact_path: entry.artifact_path.clone(),
                byte_length: entry.byte_length,
                range_start: entry.range_start,
                range_end: entry.range_end,
            });
            cache_entries.push(entry);
        }
    }

    let summary = format!(
        "Prepared {} tensors for {} (downloaded {}, cache hit {})",
        plan.tensor_assignments.len() + plan.global_tensors.len(),
        resolved.identity.summary_line(),
        mesh_core::format_bytes(bytes_downloaded),
        mesh_core::format_bytes(bytes_from_cache)
    );

    Ok(PrepareResult {
        identity: resolved.identity.clone(),
        plan: plan.clone(),
        prepared,
        cache_entries,
        bytes_downloaded,
        bytes_from_cache,
        summary,
    })
}

struct ArtifactDownloadOutcome {
    entries: Vec<ModelCacheEntry>,
    bytes_downloaded: u64,
}

async fn download_complete_artifact(
    provider: &dyn FetchSource,
    artifact: &ArtifactRef,
    cache_root: &Path,
    identity: &ModelIdentity,
    progress: &mut dyn ProgressSink,
) -> ModelResult<ArtifactDownloadOutcome> {
    let relative = complete_object_rel_path(
        &identity.provider,
        &identity.repository,
        &identity.revision,
        &artifact.relative_path,
    );
    let final_path = absolute_cache_path(cache_root, &relative);
    let partial = partial_path(&final_path);
    let meta_path = partial_meta_path(&final_path);
    ensure_parent(&final_path)?;

    progress.on_progress(DownloadProgressEvent {
        progress: ModelDownloadProgress {
            artifact_path: artifact.relative_path.clone(),
            bytes_done: 0,
            bytes_total: artifact.size_bytes,
            phase: "downloading".to_owned(),
        },
    });

    let mut attempt = 0u32;
    loop {
        attempt += 1;
        let mut last_reported = 0u64;
        let mut report = |bytes_done: u64| {
            let reached_total = artifact
                .size_bytes
                .is_some_and(|bytes_total| bytes_done >= bytes_total);
            if bytes_done.saturating_sub(last_reported) < PROGRESS_REPORT_INTERVAL_BYTES
                && !reached_total
            {
                return;
            }
            last_reported = bytes_done;
            progress.on_progress(DownloadProgressEvent {
                progress: ModelDownloadProgress {
                    artifact_path: artifact.relative_path.clone(),
                    bytes_done,
                    bytes_total: artifact.size_bytes,
                    phase: "downloading".to_owned(),
                },
            });
        };
        match provider.fetch_file(artifact, &partial, &mut report).await {
            Ok(()) => break,
            Err(error) if attempt < MAX_RANGE_ATTEMPTS && is_transient(&error) => {
                tokio::time::sleep(backoff_delay(attempt)).await;
            }
            Err(error) => return Err(error),
        }
    }

    let byte_length = file_len(&partial)?;
    if let Some(expected) = artifact.size_bytes {
        if expected != byte_length {
            let _ = std::fs::remove_file(&partial);
            return Err(ModelError::Invalid(format!(
                "complete artifact length {byte_length} != expected {expected}"
            )));
        }
    }

    if artifact.relative_path.ends_with(".safetensors") {
        validate_local_safetensors_file(&partial)?;
    }

    let digest_hex = if let Some(digest) = &artifact.digest_hex {
        let actual = crate::huggingface::sha256_file(&partial)?;
        if &actual != digest {
            let _ = std::fs::remove_file(&partial);
            return Err(ModelError::Invalid(format!(
                "digest mismatch for {}",
                artifact.relative_path
            )));
        }
        Some(actual)
    } else {
        None
    };

    publish_file(&partial, &final_path)?;
    let _ = std::fs::remove_file(meta_path);

    let now = now_unix_ms();
    let entry = ModelCacheEntry {
        entry_id: complete_entry_id(
            &identity.provider,
            &identity.repository,
            &identity.revision,
            &artifact.relative_path,
            digest_hex.as_deref(),
        ),
        provider: identity.provider.clone(),
        repository: identity.repository.clone(),
        revision: identity.revision.clone(),
        artifact_path: artifact.relative_path.clone(),
        relative_path: relative,
        byte_length,
        range_start: None,
        range_end: None,
        etag: artifact.etag.clone(),
        digest_hex,
        dtype: None,
        shape_json: None,
        state: CacheValidationState::Valid,
        reference_count: 0,
        pinned: false,
        last_used_at_unix_ms: now,
        created_at_unix_ms: now,
    };

    progress.on_progress(DownloadProgressEvent {
        progress: ModelDownloadProgress {
            artifact_path: artifact.relative_path.clone(),
            bytes_done: byte_length,
            bytes_total: Some(byte_length),
            phase: "complete".to_owned(),
        },
    });

    Ok(ArtifactDownloadOutcome {
        entries: vec![entry],
        bytes_downloaded: byte_length,
    })
}

async fn download_ranges(
    provider: &dyn FetchSource,
    artifact: &ArtifactRef,
    merged: &[(u64, u64)],
    tensors: &[TensorRecord],
    cache_root: &Path,
    identity: &ModelIdentity,
    progress: &mut dyn ProgressSink,
) -> ModelResult<ArtifactDownloadOutcome> {
    let mut entries = Vec::new();
    let mut bytes_downloaded = 0u64;
    for (start, end) in merged {
        let relative = range_object_rel_path(
            &identity.provider,
            &identity.repository,
            &identity.revision,
            &artifact.relative_path,
            *start,
            *end,
        );
        let final_path = absolute_cache_path(cache_root, &relative);
        let partial = partial_path(&final_path);
        ensure_parent(&final_path)?;

        progress.on_progress(DownloadProgressEvent {
            progress: ModelDownloadProgress {
                artifact_path: format!("{}@{start}-{end}", artifact.relative_path),
                bytes_done: 0,
                bytes_total: Some(end.saturating_sub(*start)),
                phase: "range".to_owned(),
            },
        });

        let mut attempt = 0u32;
        let bytes = loop {
            attempt += 1;
            match provider.fetch_range(artifact, *start..*end).await {
                Ok(bytes) => break bytes,
                Err(ModelError::Unsupported(message)) => {
                    return Err(ModelError::Unsupported(message));
                }
                Err(error) if attempt < MAX_RANGE_ATTEMPTS && is_transient(&error) => {
                    tokio::time::sleep(backoff_delay(attempt)).await;
                }
                Err(error) => return Err(error),
            }
        };

        if bytes.len() as u64 != end.saturating_sub(*start) {
            return Err(ModelError::Invalid(
                "range body length mismatch after validation".to_owned(),
            ));
        }

        tokio::fs::write(&partial, &bytes).await?;
        publish_file(&partial, &final_path)?;
        bytes_downloaded = bytes_downloaded.saturating_add(bytes.len() as u64);

        let covered = tensors
            .iter()
            .filter(|tensor| tensor.absolute_start >= *start && tensor.absolute_end <= *end)
            .collect::<Vec<_>>();
        let (dtype, shape_json) = if covered.len() == 1 {
            (
                Some(covered[0].dtype.clone()),
                Some(serde_json::to_string(&covered[0].shape).unwrap_or_else(|_| "[]".to_owned())),
            )
        } else {
            (None, None)
        };

        let now = now_unix_ms();
        entries.push(ModelCacheEntry {
            entry_id: range_entry_id(
                &identity.provider,
                &identity.repository,
                &identity.revision,
                &artifact.relative_path,
                *start,
                *end,
                dtype.as_deref().unwrap_or("mixed"),
                &covered
                    .first()
                    .map(|tensor| tensor.shape.clone())
                    .unwrap_or_default(),
            ),
            provider: identity.provider.clone(),
            repository: identity.repository.clone(),
            revision: identity.revision.clone(),
            artifact_path: artifact.relative_path.clone(),
            relative_path: relative,
            byte_length: bytes.len() as u64,
            range_start: Some(*start),
            range_end: Some(*end),
            etag: artifact.etag.clone(),
            digest_hex: None,
            dtype,
            shape_json,
            state: CacheValidationState::Valid,
            reference_count: 0,
            pinned: false,
            last_used_at_unix_ms: now,
            created_at_unix_ms: now,
        });
    }

    for tensor in tensors {
        let dtype = SafetensorsDtype::parse(&tensor.dtype).ok_or_else(|| {
            ModelError::Unsupported(format!("dtype {} unsupported", tensor.dtype))
        })?;
        validate_tensor_byte_length(
            dtype,
            &tensor.shape,
            tensor.absolute_start,
            tensor.absolute_end,
        )?;
    }

    Ok(ArtifactDownloadOutcome {
        entries,
        bytes_downloaded,
    })
}

fn validate_local_safetensors_file(path: &Path) -> ModelResult<()> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path)?;
    let mut prefix = [0u8; 8];
    file.read_exact(&mut prefix)?;
    let header_len = parse_header_length(&prefix)?;
    let mut header = vec![0u8; header_len as usize];
    file.read_exact(&mut header)?;
    let parsed = parse_safetensors_header(header_len, &header)?;
    let file_len = file.seek(SeekFrom::End(0))?;
    for (name, info) in parsed.tensors {
        let (start, end) = tensor_payload_absolute_range(header_len, info.data_offsets);
        if end > file_len {
            return Err(ModelError::Invalid(format!(
                "tensor {name} exceeds file length"
            )));
        }
        validate_tensor_byte_length(info.dtype, &info.shape, start, end)?;
        let _ = dtype_width_bytes(info.dtype);
    }
    Ok(())
}

fn is_transient(error: &ModelError) -> bool {
    matches!(
        error,
        ModelError::Http(_) | ModelError::Provider(_) | ModelError::Io(_)
    )
}

fn backoff_delay(attempt: u32) -> std::time::Duration {
    match attempt {
        1 => std::time::Duration::from_millis(500),
        2 => std::time::Duration::from_secs(2),
        _ => std::time::Duration::from_secs(8),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{ArtifactRecord, CanonicalManifest, TensorRecord, TensorRole};
    use mesh_core::{ModelFormat, PROVIDER_HUGGINGFACE};
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn complete_plan_hashes_stable() {
        let manifest = CanonicalManifest {
            provider: PROVIDER_HUGGINGFACE.to_owned(),
            repository: "Qwen/Qwen3-4B".to_owned(),
            revision: "0123456789abcdef0123456789abcdef01234567".to_owned(),
            adapter_id: "qwen3-dense".to_owned(),
            adapter_version: "1.0.0".to_owned(),
            model_format: ModelFormat::Safetensors,
            quantization: None,
            architecture: serde_json::json!({"num_hidden_layers": 1}),
            tensors: vec![
                TensorRecord {
                    name: "model.embed_tokens.weight".to_owned(),
                    dtype: "BF16".to_owned(),
                    shape: vec![10, 10],
                    role: TensorRole::Embedding,
                    layer_index: None,
                    artifact_path: "model.safetensors".to_owned(),
                    absolute_start: 100,
                    absolute_end: 300,
                    range_digest_hex: None,
                },
                TensorRecord {
                    name: "model.layers.0.self_attn.q_proj.weight".to_owned(),
                    dtype: "BF16".to_owned(),
                    shape: vec![10, 10],
                    role: TensorRole::Layer,
                    layer_index: Some(0),
                    artifact_path: "model.safetensors".to_owned(),
                    absolute_start: 300,
                    absolute_end: 500,
                    range_digest_hex: None,
                },
                TensorRecord {
                    name: "model.norm.weight".to_owned(),
                    dtype: "BF16".to_owned(),
                    shape: vec![10],
                    role: TensorRole::FinalNorm,
                    layer_index: None,
                    artifact_path: "model.safetensors".to_owned(),
                    absolute_start: 500,
                    absolute_end: 520,
                    range_digest_hex: None,
                },
            ],
            artifacts: vec![ArtifactRecord {
                relative_path: "model.safetensors".to_owned(),
                size_bytes: Some(520),
                etag: None,
                digest_hex: None,
            }],
            tokenizer_artifacts: vec!["tokenizer.json".to_owned()],
            tokenizer_hash: "aa".repeat(32),
            memory_estimate_bytes: 420,
        };
        let identity = crate::manifest::build_manifest_identity(&manifest).unwrap();
        let resolved = ResolvedModel {
            identity,
            manifest,
            artifacts: Vec::new(),
            local_artifacts: BTreeMap::new(),
        };
        let plan = build_complete_plan("deploy", &resolved).unwrap();
        assert_eq!(plan.first_layer, 0);
        assert_eq!(plan.last_layer_exclusive, 1);
        assert!(!plan.assignment_hash.is_empty());
    }

    fn multi_tensor_fixture() -> (Vec<u8>, CanonicalManifest, ArtifactRef, ResolvedModel) {
        // two BF16 tensors [2,2] => 8 bytes each
        let header_json = br#"{"embed":{"dtype":"BF16","shape":[2,2],"data_offsets":[0,8]},"layer0":{"dtype":"BF16","shape":[2,2],"data_offsets":[8,16]},"norm":{"dtype":"BF16","shape":[2,2],"data_offsets":[16,24]}}"#;
        let header_len = header_json.len() as u64;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&header_len.to_le_bytes());
        bytes.extend_from_slice(header_json);
        bytes.extend_from_slice(&[1u8; 24]);
        let embed = crate::safetensors::tensor_payload_absolute_range(header_len, (0, 8));
        let layer0 = crate::safetensors::tensor_payload_absolute_range(header_len, (8, 16));
        let norm = crate::safetensors::tensor_payload_absolute_range(header_len, (16, 24));
        let revision = "0123456789abcdef0123456789abcdef01234567".to_owned();
        let manifest = CanonicalManifest {
            provider: PROVIDER_HUGGINGFACE.to_owned(),
            repository: "fixture/stage".to_owned(),
            revision: revision.clone(),
            adapter_id: "qwen3-dense".to_owned(),
            adapter_version: "1.0.0".to_owned(),
            model_format: ModelFormat::Safetensors,
            quantization: None,
            architecture: serde_json::json!({
                "num_hidden_layers": 2,
                "tie_word_embeddings": true
            }),
            tensors: vec![
                TensorRecord {
                    name: "model.embed_tokens.weight".to_owned(),
                    dtype: "BF16".to_owned(),
                    shape: vec![2, 2],
                    role: TensorRole::Embedding,
                    layer_index: None,
                    artifact_path: "model.safetensors".to_owned(),
                    absolute_start: embed.0,
                    absolute_end: embed.1,
                    range_digest_hex: None,
                },
                TensorRecord {
                    name: "model.layers.0.weight".to_owned(),
                    dtype: "BF16".to_owned(),
                    shape: vec![2, 2],
                    role: TensorRole::Layer,
                    layer_index: Some(0),
                    artifact_path: "model.safetensors".to_owned(),
                    absolute_start: layer0.0,
                    absolute_end: layer0.1,
                    range_digest_hex: None,
                },
                TensorRecord {
                    name: "model.norm.weight".to_owned(),
                    dtype: "BF16".to_owned(),
                    shape: vec![2, 2],
                    role: TensorRole::FinalNorm,
                    layer_index: None,
                    artifact_path: "model.safetensors".to_owned(),
                    absolute_start: norm.0,
                    absolute_end: norm.1,
                    range_digest_hex: None,
                },
            ],
            artifacts: vec![ArtifactRecord {
                relative_path: "model.safetensors".to_owned(),
                size_bytes: Some(bytes.len() as u64),
                etag: None,
                digest_hex: None,
            }],
            tokenizer_artifacts: vec!["tokenizer.json".to_owned()],
            tokenizer_hash: "cc".repeat(32),
            memory_estimate_bytes: 24,
        };
        let artifact = ArtifactRef {
            provider: PROVIDER_HUGGINGFACE.to_owned(),
            repository: "fixture/stage".to_owned(),
            revision,
            relative_path: "model.safetensors".to_owned(),
            size_bytes: Some(bytes.len() as u64),
            etag: None,
            digest_hex: None,
        };
        let identity = crate::manifest::build_manifest_identity(&manifest).unwrap();
        let resolved = ResolvedModel {
            identity,
            manifest,
            artifacts: vec![artifact.clone()],
            local_artifacts: BTreeMap::new(),
        };
        (bytes, resolved.manifest.clone(), artifact, resolved)
    }

    #[test]
    fn layer_plan_filters_and_includes_tied_embed_for_final() {
        let (_bytes, _manifest, _artifact, resolved) = multi_tensor_fixture();
        let first = build_stage_plan(
            "deploy-first",
            &resolved,
            mesh_core::StageRole::First,
            mesh_core::LayerRange::new(0, 1).unwrap(),
        )
        .unwrap();
        assert_eq!(first.first_layer, 0);
        assert_eq!(first.last_layer_exclusive, 1);
        assert!(
            first
                .global_tensors
                .iter()
                .any(|tensor| tensor.name.contains("embed_tokens"))
        );
        assert!(
            !first
                .global_tensors
                .iter()
                .any(|tensor| tensor.name.contains("norm"))
        );
        assert_eq!(first.tensor_assignments.len(), 1);

        let final_stage = build_stage_plan(
            "deploy-final",
            &resolved,
            mesh_core::StageRole::Final,
            mesh_core::LayerRange::new(1, 2).unwrap(),
        )
        .unwrap();
        assert!(
            final_stage
                .global_tensors
                .iter()
                .any(|tensor| tensor.name.contains("embed_tokens")),
            "final tied model must include embed tensors"
        );
        assert!(
            final_stage
                .global_tensors
                .iter()
                .any(|tensor| tensor.name.contains("norm"))
        );
        assert!(final_stage.tensor_assignments.is_empty());

        let middle = build_stage_plan(
            "deploy-middle",
            &resolved,
            mesh_core::StageRole::Middle,
            mesh_core::LayerRange::new(0, 1).unwrap(),
        )
        .unwrap();
        assert!(middle.global_tensors.is_empty());
        assert_eq!(middle.tensor_assignments.len(), 1);
    }

    #[tokio::test]
    async fn prepare_layer_plan_uses_ranges_and_materializes() {
        let (bytes, _manifest, _artifact, resolved) = multi_tensor_fixture();
        // force ranges: layer-only plan covers 8/payload bytes of a larger file (<80%)
        let plan = build_stage_plan(
            "deploy-range",
            &resolved,
            mesh_core::StageRole::Middle,
            mesh_core::LayerRange::new(0, 1).unwrap(),
        )
        .unwrap();
        assert_eq!(plan.tensor_assignments.len(), 1);
        assert!(plan.disk_bytes_required > 0);

        let root = std::env::temp_dir().join(format!("mesh-stage-prep-{}", now_unix_ms()));
        let _ = std::fs::create_dir_all(&root);
        let fetch = FixtureFetch {
            bytes: Bytes::from(bytes.clone()),
            file_fetches: AtomicUsize::new(0),
            range_supported: true,
        };
        let mut progress = NoopProgress;
        let result = prepare_plan(&fetch, &resolved, &plan, &root, &[], &mut progress)
            .await
            .expect("prepare layer");
        assert!(
            result
                .prepared
                .iter()
                .any(|item| item.range_start.is_some() && item.range_end.is_some()),
            "expected range artifacts, got {:?}",
            result.prepared
        );

        let weights = materialize_stage_weight_files(&root, &result).expect("materialize");
        assert_eq!(weights.len(), 1);
        assert!(weights[0].1.is_file());
        let on_disk = std::fs::read(&weights[0].1).unwrap();
        assert!(on_disk.len() < bytes.len());
        let st = safetensors::SafeTensors::deserialize(&on_disk).expect("deserialize stage file");
        assert!(st.tensor("model.layers.0.weight").is_ok());
        assert!(st.tensor("model.embed_tokens.weight").is_err());
        let _ = std::fs::remove_dir_all(root);
    }

    struct FixtureFetch {
        bytes: Bytes,
        file_fetches: AtomicUsize,
        range_supported: bool,
    }

    impl FetchSource for FixtureFetch {
        fn fetch_file<'a>(
            &'a self,
            _artifact: &'a ArtifactRef,
            destination: &'a Path,
            on_progress: &'a mut (dyn FnMut(u64) + Send),
        ) -> BoxFuture<'a, ModelResult<()>> {
            Box::pin(async move {
                if let Some(parent) = destination.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                self.file_fetches.fetch_add(1, Ordering::Relaxed);
                std::fs::write(destination, &self.bytes)?;
                on_progress(self.bytes.len() as u64);
                Ok(())
            })
        }

        fn fetch_range<'a>(
            &'a self,
            _artifact: &'a ArtifactRef,
            range: Range<u64>,
        ) -> BoxFuture<'a, ModelResult<Bytes>> {
            Box::pin(async move {
                if !self.range_supported {
                    return Err(ModelError::Unsupported(
                        "fixture range support disabled".to_owned(),
                    ));
                }
                let start = range.start as usize;
                let end = range.end as usize;
                if end > self.bytes.len() || start >= end {
                    return Err(ModelError::Invalid(
                        "fixture range out of bounds".to_owned(),
                    ));
                }
                Ok(self.bytes.slice(start..end))
            })
        }
    }

    fn tiny_safetensors() -> (Vec<u8>, CanonicalManifest, ArtifactRef) {
        // header: one BF16 tensor shape [2,2] => 8 payload bytes
        let header_json = br#"{"t":{"dtype":"BF16","shape":[2,2],"data_offsets":[0,8]}}"#;
        let header_len = header_json.len() as u64;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&header_len.to_le_bytes());
        bytes.extend_from_slice(header_json);
        bytes.extend_from_slice(&[0u8; 8]);
        let absolute = crate::safetensors::tensor_payload_absolute_range(header_len, (0, 8));
        let revision = "0123456789abcdef0123456789abcdef01234567".to_owned();
        let manifest = CanonicalManifest {
            provider: PROVIDER_HUGGINGFACE.to_owned(),
            repository: "fixture/tiny".to_owned(),
            revision: revision.clone(),
            adapter_id: "qwen3-dense".to_owned(),
            adapter_version: "1.0.0".to_owned(),
            model_format: ModelFormat::Safetensors,
            quantization: None,
            architecture: serde_json::json!({"num_hidden_layers": 0}),
            tensors: vec![TensorRecord {
                name: "t".to_owned(),
                dtype: "BF16".to_owned(),
                shape: vec![2, 2],
                role: TensorRole::Other,
                layer_index: None,
                artifact_path: "model.safetensors".to_owned(),
                absolute_start: absolute.0,
                absolute_end: absolute.1,
                range_digest_hex: None,
            }],
            artifacts: vec![ArtifactRecord {
                relative_path: "model.safetensors".to_owned(),
                size_bytes: Some(bytes.len() as u64),
                etag: None,
                digest_hex: None,
            }],
            tokenizer_artifacts: vec!["tokenizer.json".to_owned()],
            tokenizer_hash: "bb".repeat(32),
            memory_estimate_bytes: 8,
        };
        let artifact = ArtifactRef {
            provider: PROVIDER_HUGGINGFACE.to_owned(),
            repository: "fixture/tiny".to_owned(),
            revision,
            relative_path: "model.safetensors".to_owned(),
            size_bytes: Some(bytes.len() as u64),
            etag: None,
            digest_hex: None,
        };
        (bytes, manifest, artifact)
    }

    #[derive(Default)]
    struct RecordingProgress(Vec<ModelDownloadProgress>);

    impl ProgressSink for RecordingProgress {
        fn on_progress(&mut self, event: DownloadProgressEvent) {
            self.0.push(event.progress);
        }
    }

    #[tokio::test]
    async fn prepare_uses_complete_shard_for_high_coverage() {
        let (bytes, manifest, artifact) = tiny_safetensors();
        let identity = crate::manifest::build_manifest_identity(&manifest).unwrap();
        let resolved = ResolvedModel {
            identity: identity.clone(),
            manifest,
            artifacts: vec![artifact],
            local_artifacts: BTreeMap::new(),
        };
        let plan = build_complete_plan("deploy-fixture", &resolved).unwrap();
        let root = std::env::temp_dir().join(format!("mesh-prep-{}", now_unix_ms()));
        let _ = std::fs::create_dir_all(&root);
        let fetch = FixtureFetch {
            bytes: Bytes::from(bytes.clone()),
            file_fetches: AtomicUsize::new(0),
            range_supported: false,
        };
        let mut progress = RecordingProgress::default();
        let result = prepare_plan(&fetch, &resolved, &plan, &root, &[], &mut progress)
            .await
            .expect("prepare");
        assert!(!result.cache_entries.is_empty());
        assert!(result.bytes_downloaded > 0);
        let path = root.join(&result.cache_entries[0].relative_path);
        assert!(path.exists());
        let on_disk = std::fs::read(&path).unwrap();
        assert!(!on_disk.is_empty());
        assert!(progress.0.iter().any(|event| {
            event.phase == "downloading" && event.bytes_done == bytes.len() as u64
        }));
        assert_eq!(fetch.file_fetches.load(Ordering::Relaxed), 1);

        let mut cached_progress = NoopProgress;
        let cached = prepare_plan(
            &fetch,
            &resolved,
            &plan,
            &root,
            &result.cache_entries,
            &mut cached_progress,
        )
        .await
        .expect("prepare from cache");
        assert_eq!(cached.bytes_downloaded, 0);
        assert!(cached.bytes_from_cache > 0);
        assert_eq!(fetch.file_fetches.load(Ordering::Relaxed), 1);

        let mut wrong_revision = result.cache_entries.clone();
        wrong_revision[0].revision = "ffffffffffffffffffffffffffffffffffffffff".to_owned();
        let downloaded_again = prepare_plan(
            &fetch,
            &resolved,
            &plan,
            &root,
            &wrong_revision,
            &mut cached_progress,
        )
        .await
        .expect("prepare with mismatched cache identity");
        assert!(downloaded_again.bytes_downloaded > 0);
        assert_eq!(fetch.file_fetches.load(Ordering::Relaxed), 2);
        let _ = std::fs::remove_dir_all(root);
    }
}
