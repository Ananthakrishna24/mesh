use std::collections::HashMap;
mod qwen3_stage;

use std::path::{Path, PathBuf};

use candle_core::{DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::qwen3::{Config as Qwen3Config, ModelForCausalLM};
use mesh_core::FIRST_CONTEXT_LIMIT;
use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ComputeError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Candle(#[from] candle_core::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Safetensors(#[from] safetensors::SafeTensorError),
}

pub type ComputeResult<T> = Result<T, ComputeError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    Cpu,
    Cuda,
    Metal,
}

impl BackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Cuda => "cuda",
            Self::Metal => "metal",
        }
    }
}

#[derive(Debug, Clone)]
pub struct WeightFile {
    pub artifact_path: String,
    pub absolute_path: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RawQwen3Config {
    vocab_size: usize,
    hidden_size: usize,
    intermediate_size: usize,
    num_hidden_layers: usize,
    num_attention_heads: usize,
    head_dim: Option<usize>,
    attention_bias: Option<bool>,
    num_key_value_heads: usize,
    max_position_embeddings: usize,
    sliding_window: Option<usize>,
    max_window_layers: Option<usize>,
    tie_word_embeddings: Option<bool>,
    rope_theta: Option<f64>,
    rms_norm_eps: f64,
    use_sliding_window: Option<bool>,
    hidden_act: Option<candle_nn::Activation>,
}

impl RawQwen3Config {
    fn into_config(self) -> ComputeResult<Qwen3Config> {
        if self.use_sliding_window.unwrap_or(false) || self.sliding_window.is_some() {
            return Err(ComputeError::Message(
                "sliding-window Qwen3 configs are rejected in the first profile".to_owned(),
            ));
        }
        let head_dim = self
            .head_dim
            .unwrap_or(self.hidden_size / self.num_attention_heads);
        Ok(Qwen3Config {
            vocab_size: self.vocab_size,
            hidden_size: self.hidden_size,
            intermediate_size: self.intermediate_size,
            num_hidden_layers: self.num_hidden_layers,
            num_attention_heads: self.num_attention_heads,
            head_dim,
            attention_bias: self.attention_bias.unwrap_or(false),
            num_key_value_heads: self.num_key_value_heads,
            max_position_embeddings: self.max_position_embeddings,
            sliding_window: None,
            max_window_layers: self.max_window_layers.unwrap_or(self.num_hidden_layers),
            tie_word_embeddings: self.tie_word_embeddings.unwrap_or(false),
            rope_theta: self.rope_theta.unwrap_or(1_000_000.0),
            rms_norm_eps: self.rms_norm_eps,
            use_sliding_window: false,
            hidden_act: self.hidden_act.unwrap_or(candle_nn::Activation::Silu),
        })
    }
}

pub struct LoadedQwen3 {
    pub backend: BackendKind,
    pub device: Device,
    pub dtype: DType,
    pub config: Qwen3Config,
    model: ModelForCausalLM,
    seq_len: usize,
}

impl LoadedQwen3 {
    pub fn load(
        config_json: &Path,
        weight_files: &[WeightFile],
        prefer_cuda: bool,
    ) -> ComputeResult<Self> {
        if weight_files.is_empty() {
            return Err(ComputeError::Message(
                "no weight files provided for Qwen3 load".to_owned(),
            ));
        }
        let raw: RawQwen3Config = serde_json::from_slice(&std::fs::read(config_json)?)?;
        let config = raw.into_config()?;
        if config.num_attention_heads % config.num_key_value_heads != 0 {
            return Err(ComputeError::Message(
                "num_attention_heads must be divisible by num_key_value_heads".to_owned(),
            ));
        }

        let (backend, device) = select_device(prefer_cuda)?;
        let dtype = match backend {
            BackendKind::Cpu => DType::F32,
            BackendKind::Cuda | BackendKind::Metal => DType::F16,
        };

        let paths = weight_files
            .iter()
            .map(|item| item.absolute_path.as_path())
            .collect::<Vec<_>>();
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&paths, dtype, &device)
                .map_err(ComputeError::from)?
        };
        let model = ModelForCausalLM::new(&config, vb)?;
        Ok(Self {
            backend,
            device,
            dtype,
            config,
            model,
            seq_len: 0,
        })
    }

    pub fn clear_kv_cache(&mut self) {
        self.model.clear_kv_cache();
        self.seq_len = 0;
    }

    pub fn seq_len(&self) -> usize {
        self.seq_len
    }

    pub fn context_limit(&self) -> u32 {
        FIRST_CONTEXT_LIMIT.min(self.config.max_position_embeddings as u32)
    }

    pub fn vocab_size(&self) -> u32 {
        self.config.vocab_size as u32
    }

    pub fn num_layers(&self) -> u32 {
        self.config.num_hidden_layers as u32
    }

    pub fn num_kv_heads(&self) -> u32 {
        self.config.num_key_value_heads as u32
    }

    pub fn head_dim(&self) -> u32 {
        self.config.head_dim as u32
    }

    pub fn prefill_logits(&mut self, token_ids: &[u32]) -> ComputeResult<Vec<f32>> {
        if token_ids.is_empty() {
            return Err(ComputeError::Message("prefill requires tokens".to_owned()));
        }
        self.clear_kv_cache();
        let input = Tensor::new(token_ids, &self.device)?.unsqueeze(0)?;
        let logits = self.model.forward(&input, 0)?;
        self.seq_len = token_ids.len();
        logits_to_vec_f32(&logits)
    }

    pub fn decode_logits(&mut self, token_id: u32) -> ComputeResult<Vec<f32>> {
        let input = Tensor::new(&[token_id], &self.device)?.unsqueeze(0)?;
        let offset = self.seq_len;
        let logits = self.model.forward(&input, offset)?;
        self.seq_len = self.seq_len.saturating_add(1);
        logits_to_vec_f32(&logits)
    }
}

pub(crate) fn select_device(prefer_cuda: bool) -> ComputeResult<(BackendKind, Device)> {
    if prefer_cuda {
        #[cfg(feature = "cuda")]
        {
            match Device::new_cuda(0) {
                Ok(device) => return Ok((BackendKind::Cuda, device)),
                Err(error) => {
                    tracing::warn!(%error, "CUDA device unavailable; falling back");
                }
            }
        }
        #[cfg(feature = "metal")]
        {
            match Device::new_metal(0) {
                Ok(device) => return Ok((BackendKind::Metal, device)),
                Err(error) => {
                    tracing::warn!(%error, "Metal device unavailable; falling back");
                }
            }
        }
    }
    Ok((BackendKind::Cpu, Device::Cpu))
}

pub(crate) fn logits_to_vec_f32(logits: &Tensor) -> ComputeResult<Vec<f32>> {
    let logits = logits.squeeze(0)?.squeeze(0)?.to_dtype(DType::F32)?;
    let values = logits.to_vec1::<f32>()?;
    Ok(values)
}

pub fn group_complete_weight_files(
    cache_root: &Path,
    prepared: &[(String, PathBuf, Option<u64>, Option<u64>)],
) -> ComputeResult<Vec<WeightFile>> {
    let mut by_artifact: HashMap<String, PathBuf> = HashMap::new();
    for (artifact_path, relative_path, range_start, range_end) in prepared {
        if range_start.is_some() || range_end.is_some() {
            continue;
        }
        by_artifact
            .entry(artifact_path.clone())
            .or_insert_with(|| cache_root.join(relative_path));
    }
    if by_artifact.is_empty() {
        return Err(ComputeError::Message(
            "complete-stage load requires whole-shard weight files in cache".to_owned(),
        ));
    }
    let mut files = by_artifact
        .into_iter()
        .map(|(artifact_path, absolute_path)| WeightFile {
            artifact_path,
            absolute_path,
        })
        .collect::<Vec<_>>();
    files.sort_by(|left, right| left.artifact_path.cmp(&right.artifact_path));
    Ok(files)
}

pub use qwen3_stage::Qwen3Stage;

pub fn crate_name() -> &'static str {
    "mesh-compute"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_weight_list() {
        let err = group_complete_weight_files(Path::new("/tmp"), &[]).unwrap_err();
        assert!(err.to_string().contains("whole-shard"));
    }
}
