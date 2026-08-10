mod error;
mod manifest;
mod safetensors;
mod validate;

pub use error::{ModelError, ModelResult};
pub use manifest::{
    ArtifactRecord, CanonicalManifest, TensorRecord, TensorRole, build_manifest_identity,
    canonical_manifest_bytes, hash_bytes_hex, manifest_hash_hex, qwen3_dense_adapter_ids,
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
