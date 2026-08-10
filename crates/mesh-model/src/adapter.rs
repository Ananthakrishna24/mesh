use std::collections::{BTreeMap, BTreeSet};

use mesh_core::{
    ADAPTER_QWEN3_DENSE, ADAPTER_QWEN3_DENSE_VERSION, ModelFormat, PROVIDER_HUGGINGFACE,
};
use serde_json::Value;

use crate::manifest::{ArtifactRecord, CanonicalManifest, TensorRecord, TensorRole};
use crate::safetensors::{SafetensorsHeader, dtype_width_bytes, tensor_payload_absolute_range};
use crate::{ModelError, ModelResult};

#[derive(Debug, Clone)]
pub struct WeightShard {
    pub relative_path: String,
    pub size_bytes: Option<u64>,
    pub etag: Option<String>,
    pub digest_hex: Option<String>,
    pub header: SafetensorsHeader,
}

#[derive(Debug, Clone)]
pub struct AdapterInputs {
    pub repository: String,
    pub revision: String,
    pub config: Value,
    pub tokenizer_artifacts: Vec<String>,
    pub tokenizer_hash: String,
    pub shards: Vec<WeightShard>,
    pub extra_artifacts: Vec<ArtifactRecord>,
}

pub fn build_qwen3_dense_manifest(inputs: AdapterInputs) -> ModelResult<CanonicalManifest> {
    let model_type = inputs
        .config
        .get("model_type")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if !model_type.contains("qwen3") {
        return Err(ModelError::Unsupported(format!(
            "adapter qwen3-dense cannot map model_type={model_type}"
        )));
    }

    let num_layers = required_u64(&inputs.config, "num_hidden_layers")? as u32;
    let _hidden = required_u64(&inputs.config, "hidden_size")?;
    let architectures = inputs
        .config
        .get("architectures")
        .cloned()
        .unwrap_or(Value::Null);

    let mut tensor_to_shard: BTreeMap<String, &WeightShard> = BTreeMap::new();
    for shard in &inputs.shards {
        for name in shard.header.tensors.keys() {
            if tensor_to_shard.insert(name.clone(), shard).is_some() {
                return Err(ModelError::Invalid(format!(
                    "tensor {name} appears in multiple shards"
                )));
            }
        }
    }

    let mut tensors = Vec::new();
    let mut memory_estimate_bytes = 0u64;
    let mut seen_layers = BTreeSet::new();

    for (name, shard) in &tensor_to_shard {
        let info = shard
            .header
            .tensors
            .get(name.as_str())
            .expect("tensor present");
        let (absolute_start, absolute_end) =
            tensor_payload_absolute_range(shard.header.header_length, info.data_offsets);
        let expected = info
            .shape
            .iter()
            .try_fold(dtype_width_bytes(info.dtype), |acc, dim| {
                acc.checked_mul(*dim)
                    .ok_or_else(|| ModelError::Invalid(format!("tensor {name} shape overflow")))
            })?;
        if absolute_end.saturating_sub(absolute_start) != expected {
            return Err(ModelError::Invalid(format!(
                "tensor {name} byte length mismatch"
            )));
        }
        let (role, layer_index) = classify_tensor(name, num_layers)?;
        if let Some(layer) = layer_index {
            seen_layers.insert(layer);
        }
        memory_estimate_bytes = memory_estimate_bytes.saturating_add(expected);
        tensors.push(TensorRecord {
            name: name.clone(),
            dtype: info.dtype.as_str().to_owned(),
            shape: info.shape.clone(),
            role,
            layer_index,
            artifact_path: shard.relative_path.clone(),
            absolute_start,
            absolute_end,
            range_digest_hex: None,
        });
    }

    if seen_layers.len() as u32 != num_layers {
        return Err(ModelError::Invalid(format!(
            "expected tensors for {num_layers} layers, found {}",
            seen_layers.len()
        )));
    }
    for layer in 0..num_layers {
        if !seen_layers.contains(&layer) {
            return Err(ModelError::Invalid(format!(
                "missing tensors for layer {layer}"
            )));
        }
    }

    let has_embed = tensors
        .iter()
        .any(|tensor| tensor.role == TensorRole::Embedding);
    let has_norm = tensors
        .iter()
        .any(|tensor| tensor.role == TensorRole::FinalNorm);
    if !has_embed {
        return Err(ModelError::Invalid(
            "missing embedding tensors for qwen3-dense".to_owned(),
        ));
    }
    if !has_norm {
        return Err(ModelError::Invalid(
            "missing final norm tensors for qwen3-dense".to_owned(),
        ));
    }

    let mut artifacts = inputs
        .shards
        .iter()
        .map(|shard| ArtifactRecord {
            relative_path: shard.relative_path.clone(),
            size_bytes: shard.size_bytes,
            etag: shard.etag.clone(),
            digest_hex: shard.digest_hex.clone(),
        })
        .collect::<Vec<_>>();
    artifacts.extend(inputs.extra_artifacts);
    for tokenizer in &inputs.tokenizer_artifacts {
        if !artifacts
            .iter()
            .any(|artifact| artifact.relative_path == *tokenizer)
        {
            artifacts.push(ArtifactRecord {
                relative_path: tokenizer.clone(),
                size_bytes: None,
                etag: None,
                digest_hex: None,
            });
        }
    }

    let architecture = serde_json::json!({
        "model_type": model_type,
        "num_hidden_layers": num_layers,
        "hidden_size": _hidden,
        "architectures": architectures,
        "tie_word_embeddings": inputs.config.get("tie_word_embeddings").cloned().unwrap_or(Value::Bool(false)),
        "vocab_size": inputs.config.get("vocab_size").cloned().unwrap_or(Value::Null),
        "num_attention_heads": inputs.config.get("num_attention_heads").cloned().unwrap_or(Value::Null),
        "num_key_value_heads": inputs.config.get("num_key_value_heads").cloned().unwrap_or(Value::Null),
        "intermediate_size": inputs.config.get("intermediate_size").cloned().unwrap_or(Value::Null),
        "rms_norm_eps": inputs.config.get("rms_norm_eps").cloned().unwrap_or(Value::Null),
        "max_position_embeddings": inputs.config.get("max_position_embeddings").cloned().unwrap_or(Value::Null),
    });

    Ok(CanonicalManifest {
        provider: PROVIDER_HUGGINGFACE.to_owned(),
        repository: inputs.repository,
        revision: inputs.revision,
        adapter_id: ADAPTER_QWEN3_DENSE.to_owned(),
        adapter_version: ADAPTER_QWEN3_DENSE_VERSION.to_owned(),
        model_format: ModelFormat::Safetensors,
        quantization: None,
        architecture,
        tensors,
        artifacts,
        tokenizer_artifacts: inputs.tokenizer_artifacts,
        tokenizer_hash: inputs.tokenizer_hash,
        memory_estimate_bytes,
    }
    .sorted())
}

fn classify_tensor(name: &str, num_layers: u32) -> ModelResult<(TensorRole, Option<u32>)> {
    if name.starts_with("model.embed_tokens.") || name == "model.embed_tokens.weight" {
        return Ok((TensorRole::Embedding, None));
    }
    if name.starts_with("model.norm.") || name == "model.norm.weight" {
        return Ok((TensorRole::FinalNorm, None));
    }
    if name.starts_with("lm_head.") || name == "lm_head.weight" {
        return Ok((TensorRole::LmHead, None));
    }
    if let Some(rest) = name.strip_prefix("model.layers.") {
        let (index_text, _) = rest
            .split_once('.')
            .ok_or_else(|| ModelError::Invalid(format!("unrecognized layer tensor {name}")))?;
        let index: u32 = index_text
            .parse()
            .map_err(|_| ModelError::Invalid(format!("invalid layer index in {name}")))?;
        if index >= num_layers {
            return Err(ModelError::Invalid(format!(
                "layer index {index} exceeds num_hidden_layers {num_layers}"
            )));
        }
        return Ok((TensorRole::Layer, Some(index)));
    }
    Ok((TensorRole::Other, None))
}

fn required_u64(config: &Value, key: &str) -> ModelResult<u64> {
    config
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| ModelError::Invalid(format!("config missing numeric field {key}")))
}
