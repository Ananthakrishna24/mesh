mod adapter;
mod cache;
mod download;
mod error;
mod huggingface;
mod manifest;
mod provider;
mod safetensors;
mod validate;

pub use adapter::{AdapterInputs, WeightShard, build_qwen3_dense_manifest};
pub use cache::{
    PartialMeta, absolute_cache_path, artifact_path_hash, cleanup_incomplete, complete_entry_id,
    complete_object_rel_path, default_hf_provider, ensure_parent, file_len, now_ms,
    partial_meta_path, partial_path, publish_file, range_entry_id, range_object_rel_path,
    repository_hash, should_evict_for_space, validate_entry_file,
};
pub use download::{
    DownloadProgressEvent, FetchSource, NoopProgress, PrepareResult, ProgressSink,
    build_complete_plan, build_layer_plan, build_stage_plan, materialize_stage_weight_files,
    net_disk_bytes_required, prepare_plan, resolved_tie_word_embeddings, stage_plan_flags,
};
pub use error::{ModelError, ModelResult};
pub use huggingface::{HuggingFaceProvider, identity_summary, sha256_file};
pub use manifest::{
    ArtifactRecord, CanonicalManifest, TensorRecord, TensorRole, build_manifest_identity,
    canonical_manifest_bytes, hash_bytes_hex, manifest_hash_hex, qwen3_dense_adapter_ids,
};
pub use provider::{
    ArtifactMetadata, ArtifactRef, NodeModelPlan, PreparedArtifact, ResolvedModel, TensorAssignment,
};
pub use safetensors::{
    SafetensorsDtype, SafetensorsHeader, SafetensorsTensorInfo, default_merge_ranges,
    dtype_width_bytes, merge_byte_ranges, parse_header_length, parse_safetensors_header,
    tensor_payload_absolute_range,
};
pub use validate::{
    ContentRange, RangeValidation, parse_content_range_header, validate_content_range,
    validate_tensor_byte_length,
};

pub fn crate_name() -> &'static str {
    "mesh-model"
}
