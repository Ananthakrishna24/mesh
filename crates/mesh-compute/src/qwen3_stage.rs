use std::path::Path;
use std::sync::Arc;

use candle_core::{DType, Device, Module, Tensor};
use candle_nn::kv_cache::ConcatKvCache;
use candle_nn::VarBuilder;
use candle_transformers::models::qwen3::Config as Qwen3Config;
use candle_transformers::models::with_tracing::{
    linear_b, linear_no_bias, Embedding, Linear, RmsNorm,
};
use candle_transformers::utils::repeat_kv;
use mesh_core::{LayerRange, StageRole, FIRST_CONTEXT_LIMIT};

use crate::{
    group_complete_weight_files, select_device, BackendKind, ComputeError, ComputeResult, WeightFile,
};

#[derive(Debug, Clone)]
struct Qwen3RotaryEmbedding {
    sin: Tensor,
    cos: Tensor,
}

impl Qwen3RotaryEmbedding {
    fn new(dtype: DType, cfg: &Qwen3Config, dev: &Device) -> ComputeResult<Self> {
        let dim = cfg.head_dim;
        let max_seq_len = cfg.max_position_embeddings;
        let inv_freq: Vec<_> = (0..dim)
            .step_by(2)
            .map(|i| 1f32 / cfg.rope_theta.powf(i as f64 / dim as f64) as f32)
            .collect();
        let inv_freq_len = inv_freq.len();
        let inv_freq = Tensor::from_vec(inv_freq, (1, inv_freq_len), dev)?.to_dtype(DType::F32)?;
        let t = Tensor::arange(0u32, max_seq_len as u32, dev)?
            .to_dtype(DType::F32)?
            .reshape((max_seq_len, 1))?;
        let freqs = t.matmul(&inv_freq)?;
        Ok(Self {
            sin: freqs.sin()?.to_dtype(dtype)?,
            cos: freqs.cos()?.to_dtype(dtype)?,
        })
    }

    fn apply(&self, q: &Tensor, k: &Tensor, offset: usize) -> ComputeResult<(Tensor, Tensor)> {
        let (_, _, seq_len, _) = q.dims4()?;
        let cos = self.cos.narrow(0, offset, seq_len)?;
        let sin = self.sin.narrow(0, offset, seq_len)?;
        let q_embed = candle_nn::rotary_emb::rope(&q.contiguous()?, &cos, &sin)?;
        let k_embed = candle_nn::rotary_emb::rope(&k.contiguous()?, &cos, &sin)?;
        Ok((q_embed, k_embed))
    }
}

#[derive(Debug, Clone)]
struct Qwen3Mlp {
    gate_proj: Linear,
    up_proj: Linear,
    down_proj: Linear,
    act_fn: candle_nn::Activation,
}

impl Qwen3Mlp {
    fn new(cfg: &Qwen3Config, vb: VarBuilder) -> ComputeResult<Self> {
        Ok(Self {
            gate_proj: linear_no_bias(cfg.hidden_size, cfg.intermediate_size, vb.pp("gate_proj"))?,
            up_proj: linear_no_bias(cfg.hidden_size, cfg.intermediate_size, vb.pp("up_proj"))?,
            down_proj: linear_no_bias(cfg.intermediate_size, cfg.hidden_size, vb.pp("down_proj"))?,
            act_fn: cfg.hidden_act,
        })
    }
}

impl Module for Qwen3Mlp {
    fn forward(&self, x: &Tensor) -> candle_core::Result<Tensor> {
        let lhs = x.apply(&self.gate_proj)?.apply(&self.act_fn)?;
        let rhs = x.apply(&self.up_proj)?;
        (lhs * rhs)?.apply(&self.down_proj)
    }
}

#[derive(Debug, Clone)]
struct Qwen3Attention {
    q_proj: Linear,
    k_proj: Linear,
    v_proj: Linear,
    o_proj: Linear,
    q_norm: RmsNorm,
    k_norm: RmsNorm,
    num_heads: usize,
    num_kv_heads: usize,
    num_kv_groups: usize,
    head_dim: usize,
    hidden_size: usize,
    rotary_emb: Arc<Qwen3RotaryEmbedding>,
    kv_cache: ConcatKvCache,
}

impl Qwen3Attention {
    fn new(
        cfg: &Qwen3Config,
        rotary_emb: Arc<Qwen3RotaryEmbedding>,
        vb: VarBuilder,
    ) -> ComputeResult<Self> {
        if cfg.use_sliding_window {
            return Err(ComputeError::Message(
                "sliding window is not supported".to_owned(),
            ));
        }
        let head_dim = cfg.head_dim;
        let num_heads = cfg.num_attention_heads;
        let num_kv_heads = cfg.num_key_value_heads;
        let num_kv_groups = num_heads / num_kv_heads;
        let q_proj = linear_b(
            cfg.hidden_size,
            num_heads * head_dim,
            cfg.attention_bias,
            vb.pp("q_proj"),
        )?;
        let k_proj = linear_b(
            cfg.hidden_size,
            num_kv_heads * head_dim,
            cfg.attention_bias,
            vb.pp("k_proj"),
        )?;
        let v_proj = linear_b(
            cfg.hidden_size,
            num_kv_heads * head_dim,
            cfg.attention_bias,
            vb.pp("v_proj"),
        )?;
        let o_proj = linear_b(
            num_heads * head_dim,
            cfg.hidden_size,
            cfg.attention_bias,
            vb.pp("o_proj"),
        )?;
        let q_norm = RmsNorm::new(head_dim, cfg.rms_norm_eps, vb.pp("q_norm"))?;
        let k_norm = RmsNorm::new(head_dim, cfg.rms_norm_eps, vb.pp("k_norm"))?;
        let hidden_size = head_dim * cfg.num_attention_heads;
        Ok(Self {
            q_proj,
            k_proj,
            v_proj,
            o_proj,
            q_norm,
            k_norm,
            num_heads,
            num_kv_heads,
            num_kv_groups,
            head_dim,
            hidden_size,
            rotary_emb,
            kv_cache: ConcatKvCache::new(2),
        })
    }

    fn forward(
        &mut self,
        x: &Tensor,
        attn_mask: Option<&Tensor>,
        offset: usize,
    ) -> ComputeResult<Tensor> {
        let (b, l, _) = x.dims3()?;
        let q = self.q_proj.forward(x)?;
        let k = self.k_proj.forward(x)?;
        let v = self.v_proj.forward(x)?;
        let q = q
            .reshape((b, l, self.num_heads, self.head_dim))?
            .transpose(1, 2)?;
        let k = k
            .reshape((b, l, self.num_kv_heads, self.head_dim))?
            .transpose(1, 2)?;
        let v = v
            .reshape((b, l, self.num_kv_heads, self.head_dim))?
            .transpose(1, 2)?;
        let q_flat = self.q_norm.forward(&q.flatten(0, 2)?)?;
        let k_flat = self.k_norm.forward(&k.flatten(0, 2)?)?;
        let q = q_flat.reshape((b, self.num_heads, l, self.head_dim))?;
        let k = k_flat.reshape((b, self.num_kv_heads, l, self.head_dim))?;
        let (q, k) = self.rotary_emb.apply(&q, &k, offset)?;
        let (k, v) = self.kv_cache.append(&k, &v)?;
        let on_cpu = x.device().is_cpu();
        if on_cpu {
            return self.forward_cpu_flash_attn(&q, &k, &v, offset, b, l);
        }
        self.forward_standard_attn(&q, &k, &v, attn_mask, b, l)
    }

    fn forward_cpu_flash_attn(
        &self,
        q: &Tensor,
        k: &Tensor,
        v: &Tensor,
        offset: usize,
        b: usize,
        l: usize,
    ) -> ComputeResult<Tensor> {
        use candle_nn::attention::{flash_attn, AttnMask};
        let q = q.transpose(1, 2)?.contiguous()?;
        let k = k.transpose(1, 2)?.contiguous()?;
        let v = v.transpose(1, 2)?.contiguous()?;
        let scale = 1.0 / (self.head_dim as f32).sqrt();
        let ctx = match q.dtype() {
            DType::F32 => flash_attn::<f32>(
                &q,
                &k,
                &v,
                scale,
                AttnMask::causal_with_offset(offset),
                None,
                None,
            )?,
            other => {
                let q_f32 = q.to_dtype(DType::F32)?;
                let k_f32 = k.to_dtype(DType::F32)?;
                let v_f32 = v.to_dtype(DType::F32)?;
                let ctx_f32 = flash_attn::<f32>(
                    &q_f32,
                    &k_f32,
                    &v_f32,
                    scale,
                    AttnMask::causal_with_offset(offset),
                    None,
                    None,
                )?;
                ctx_f32.to_dtype(other)?
            }
        };
        let ctx = ctx.transpose(1, 2)?;
        Ok(ctx.reshape((b, l, self.hidden_size))?.apply(&self.o_proj)?)
    }

    fn forward_standard_attn(
        &self,
        q: &Tensor,
        k: &Tensor,
        v: &Tensor,
        attn_mask: Option<&Tensor>,
        b: usize,
        l: usize,
    ) -> ComputeResult<Tensor> {
        let k = repeat_kv(k.clone(), self.num_kv_groups)?.contiguous()?;
        let v = repeat_kv(v.clone(), self.num_kv_groups)?.contiguous()?;
        let scale = 1.0 / (self.head_dim as f64).sqrt();
        let mut scores = (q.matmul(&k.transpose(2, 3)?)? * scale)?;
        if let Some(mask) = attn_mask {
            scores = scores.broadcast_add(mask)?;
        }
        let probs = candle_nn::ops::softmax_last_dim(&scores)?;
        let ctx = probs.matmul(&v)?;
        Ok(ctx
            .transpose(1, 2)?
            .reshape((b, l, self.hidden_size))?
            .apply(&self.o_proj)?)
    }

    fn clear_kv_cache(&mut self) {
        self.kv_cache.reset();
    }
}

#[derive(Debug, Clone)]
struct DecoderLayer {
    self_attn: Qwen3Attention,
    mlp: Qwen3Mlp,
    ln1: RmsNorm,
    ln2: RmsNorm,
}

impl DecoderLayer {
    fn new(
        cfg: &Qwen3Config,
        rotary: Arc<Qwen3RotaryEmbedding>,
        vb: VarBuilder,
    ) -> ComputeResult<Self> {
        Ok(Self {
            self_attn: Qwen3Attention::new(cfg, rotary, vb.pp("self_attn"))?,
            mlp: Qwen3Mlp::new(cfg, vb.pp("mlp"))?,
            ln1: RmsNorm::new(cfg.hidden_size, cfg.rms_norm_eps, vb.pp("input_layernorm"))?,
            ln2: RmsNorm::new(
                cfg.hidden_size,
                cfg.rms_norm_eps,
                vb.pp("post_attention_layernorm"),
            )?,
        })
    }

    fn forward(
        &mut self,
        x: &Tensor,
        mask: Option<&Tensor>,
        offset: usize,
    ) -> ComputeResult<Tensor> {
        let h = self.ln1.forward(x)?;
        let h = self.self_attn.forward(&h, mask, offset)?;
        let x = (x + h)?;
        let h2 = self.ln2.forward(&x)?;
        let h2 = h2.apply(&self.mlp)?;
        Ok((x + h2)?)
    }

    fn clear_kv_cache(&mut self) {
        self.self_attn.clear_kv_cache();
    }
}

pub struct Qwen3Stage {
    pub role: StageRole,
    pub layer_range: LayerRange,
    pub backend: BackendKind,
    pub device: Device,
    pub dtype: DType,
    pub config: Qwen3Config,
    embedding: Option<Embedding>,
    layers: Vec<DecoderLayer>,
    final_norm: Option<RmsNorm>,
    lm_head: Option<Linear>,
    seq_len: usize,
}

impl Qwen3Stage {
    pub fn load(
        config_json: &Path,
        weight_files: &[WeightFile],
        role: StageRole,
        layer_range: LayerRange,
        prefer_cuda: bool,
    ) -> ComputeResult<Self> {
        if weight_files.is_empty() {
            return Err(ComputeError::Message(
                "no weight files provided for Qwen3 stage load".to_owned(),
            ));
        }
        if layer_range.len() == 0 {
            return Err(ComputeError::Message(
                "stage layer range must be non-empty".to_owned(),
            ));
        }
        let raw: crate::RawQwen3Config = serde_json::from_slice(&std::fs::read(config_json)?)?;
        let config = raw.into_config()?;
        if layer_range.end as usize > config.num_hidden_layers {
            return Err(ComputeError::Message(format!(
                "layer range end {} exceeds model layers {}",
                layer_range.end, config.num_hidden_layers
            )));
        }
        if config.num_attention_heads % config.num_key_value_heads != 0 {
            return Err(ComputeError::Message(
                "num_attention_heads must be divisible by num_key_value_heads".to_owned(),
            ));
        }
        validate_role_range(role, &layer_range, config.num_hidden_layers as u32)?;

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
            VarBuilder::from_mmaped_safetensors(&paths, dtype, &device).map_err(ComputeError::from)?
        };
        if vb.dtype() == DType::F64 {
            return Err(ComputeError::Message(
                "Qwen3 does not support f64 weights".to_owned(),
            ));
        }

        let rotary = Arc::new(Qwen3RotaryEmbedding::new(dtype, &config, &device)?);
        let mut layers = Vec::with_capacity(layer_range.len() as usize);
        let vb_layers = vb.pp("model.layers");
        for layer_idx in layer_range.start..layer_range.end {
            layers.push(DecoderLayer::new(
                &config,
                rotary.clone(),
                vb_layers.pp(layer_idx as usize),
            )?);
        }

        let embedding = if role.owns_embeddings() {
            Some(Embedding::new(
                config.vocab_size,
                config.hidden_size,
                vb.pp("model.embed_tokens"),
            )?)
        } else {
            None
        };

        let final_norm = if role.owns_output_head() {
            Some(RmsNorm::new(
                config.hidden_size,
                config.rms_norm_eps,
                vb.pp("model.norm"),
            )?)
        } else {
            None
        };

        let lm_head = if role.owns_output_head() {
            Some(if config.tie_word_embeddings {
                let weights = match &embedding {
                    Some(embed) => embed.embeddings().clone(),
                    None => {
                        let tied = Embedding::new(
                            config.vocab_size,
                            config.hidden_size,
                            vb.pp("model.embed_tokens"),
                        )?;
                        tied.embeddings().clone()
                    }
                };
                Linear::from_weights(weights, None)
            } else {
                linear_no_bias(config.hidden_size, config.vocab_size, vb.pp("lm_head"))?
            })
        } else {
            None
        };

        Ok(Self {
            role,
            layer_range,
            backend,
            device,
            dtype,
            config,
            embedding,
            layers,
            final_norm,
            lm_head,
            seq_len: 0,
        })
    }

    pub fn load_from_prepared(
        config_json: &Path,
        cache_root: &Path,
        prepared: &[(String, std::path::PathBuf, Option<u64>, Option<u64>)],
        role: StageRole,
        layer_range: LayerRange,
        prefer_cuda: bool,
    ) -> ComputeResult<Self> {
        let weight_files = group_complete_weight_files(cache_root, prepared)?;
        Self::load(config_json, &weight_files, role, layer_range, prefer_cuda)
    }

    pub fn clear_kv_cache(&mut self) {
        for layer in &mut self.layers {
            layer.clear_kv_cache();
        }
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

    pub fn num_layers_owned(&self) -> u32 {
        self.layer_range.len()
    }

    pub fn num_kv_heads(&self) -> u32 {
        self.config.num_key_value_heads as u32
    }

    pub fn head_dim(&self) -> u32 {
        self.config.head_dim as u32
    }

    pub fn hidden_size(&self) -> u32 {
        self.config.hidden_size as u32
    }

    pub fn forward_tokens(&mut self, token_ids: &[u32], offset: usize) -> ComputeResult<Tensor> {
        if !self.role.accepts_token_ids() {
            return Err(ComputeError::Message(format!(
                "stage role {} rejects token ids",
                self.role.as_str()
            )));
        }
        if token_ids.is_empty() {
            return Err(ComputeError::Message(
                "token forward requires tokens".to_owned(),
            ));
        }
        let input = Tensor::new(token_ids, &self.device)?.unsqueeze(0)?;
        let embed = self
            .embedding
            .as_ref()
            .ok_or_else(|| ComputeError::Message("missing embedding weights".to_owned()))?;
        let hidden = embed.forward(&input)?;
        self.forward_hidden(&hidden, offset)
    }

    pub fn forward_activation(
        &mut self,
        activation: &Tensor,
        offset: usize,
    ) -> ComputeResult<Tensor> {
        if self.role.accepts_token_ids() && !matches!(self.role, StageRole::Complete) {
            return Err(ComputeError::Message(
                "first stage expects token ids, not activations".to_owned(),
            ));
        }
        self.forward_hidden(activation, offset)
    }

    pub fn prefill_tokens(&mut self, token_ids: &[u32]) -> ComputeResult<Tensor> {
        self.clear_kv_cache();
        let output = self.forward_tokens(token_ids, 0)?;
        self.seq_len = token_ids.len();
        Ok(output)
    }

    pub fn decode_token(&mut self, token_id: u32) -> ComputeResult<Tensor> {
        let offset = self.seq_len;
        let output = self.forward_tokens(&[token_id], offset)?;
        self.seq_len = self.seq_len.saturating_add(1);
        Ok(output)
    }

    pub fn prefill_activation(&mut self, activation: &Tensor) -> ComputeResult<Tensor> {
        self.clear_kv_cache();
        let (_batch, seq, _hidden) = activation.dims3()?;
        let output = self.forward_activation(activation, 0)?;
        self.seq_len = seq;
        Ok(output)
    }

    pub fn decode_activation(&mut self, activation: &Tensor) -> ComputeResult<Tensor> {
        let (_batch, seq, _hidden) = activation.dims3()?;
        if seq != 1 {
            return Err(ComputeError::Message(
                "decode activation sequence length must be 1".to_owned(),
            ));
        }
        let offset = self.seq_len;
        let output = self.forward_activation(activation, offset)?;
        self.seq_len = self.seq_len.saturating_add(1);
        Ok(output)
    }

    pub fn logits_from_hidden(&self, hidden: &Tensor) -> ComputeResult<Vec<f32>> {
        if !self.role.emits_logits() {
            return Err(ComputeError::Message(format!(
                "stage role {} does not emit logits",
                self.role.as_str()
            )));
        }
        let (_batch, seq, _hidden) = hidden.dims3()?;
        let last = hidden.narrow(1, seq - 1, 1)?;
        let head = self
            .lm_head
            .as_ref()
            .ok_or_else(|| ComputeError::Message("missing lm_head weights".to_owned()))?;
        let logits = last.apply(head)?;
        crate::logits_to_vec_f32(&logits)
    }

    pub fn activation_to_fp16_bytes(&self, activation: &Tensor) -> ComputeResult<Vec<u8>> {
        let contiguous = activation.contiguous()?.to_dtype(DType::F16)?;
        let values = contiguous.flatten_all()?.to_vec1::<half::f16>()?;
        let mut bytes = Vec::with_capacity(values.len() * 2);
        for value in values {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        Ok(bytes)
    }

    pub fn activation_from_fp16_bytes(
        &self,
        bytes: &[u8],
        batch: usize,
        sequence: usize,
        hidden: usize,
    ) -> ComputeResult<Tensor> {
        let expected = batch
            .checked_mul(sequence)
            .and_then(|v| v.checked_mul(hidden))
            .and_then(|v| v.checked_mul(2))
            .ok_or_else(|| ComputeError::Message("activation byte length overflow".to_owned()))?;
        if bytes.len() != expected {
            return Err(ComputeError::Message(format!(
                "activation payload len {} != expected {expected}",
                bytes.len()
            )));
        }
        let mut values = Vec::with_capacity(expected / 2);
        for chunk in bytes.chunks_exact(2) {
            values.push(half::f16::from_le_bytes([chunk[0], chunk[1]]));
        }
        let tensor = Tensor::from_vec(values, (batch, sequence, hidden), &self.device)?;
        Ok(tensor.to_dtype(self.dtype)?)
    }

    fn forward_hidden(&mut self, hidden: &Tensor, offset: usize) -> ComputeResult<Tensor> {
        let (b, l, h) = hidden.dims3()?;
        if h != self.config.hidden_size {
            return Err(ComputeError::Message(format!(
                "hidden size {h} != config {}",
                self.config.hidden_size
            )));
        }
        let needs_mask = !self.device.is_cpu() && l > 1;
        let causal = if needs_mask {
            Some(self.causal_mask(b, l, offset)?)
        } else {
            None
        };
        let mut h_tensor = hidden.clone();
        for layer in &mut self.layers {
            h_tensor = layer.forward(&h_tensor, causal.as_ref(), offset)?;
        }
        if let Some(norm) = &self.final_norm {
            h_tensor = norm.forward(&h_tensor)?;
        }
        let _ = b;
        Ok(h_tensor)
    }

    fn causal_mask(&self, b: usize, tgt: usize, offset: usize) -> ComputeResult<Tensor> {
        let minf = f32::NEG_INFINITY;
        let mask: Vec<_> = (0..tgt)
            .flat_map(|i| {
                (0..(tgt + offset)).map(move |j| {
                    if j <= i + offset {
                        0.
                    } else {
                        minf
                    }
                })
            })
            .collect();
        Ok(
            Tensor::from_slice(&mask, (b, 1, tgt, tgt + offset), &self.device)?
                .to_dtype(self.dtype)?,
        )
    }
}

fn validate_role_range(
    role: StageRole,
    layer_range: &LayerRange,
    num_layers: u32,
) -> ComputeResult<()> {
    match role {
        StageRole::Complete => {
            if layer_range.start != 0 || layer_range.end != num_layers {
                return Err(ComputeError::Message(
                    "complete stage must own every layer".to_owned(),
                ));
            }
        }
        StageRole::First => {
            if layer_range.start != 0 {
                return Err(ComputeError::Message(
                    "first stage must start at layer 0".to_owned(),
                ));
            }
            if layer_range.end >= num_layers {
                return Err(ComputeError::Message(
                    "first stage cannot own the final layer exclusively in multi-stage mode"
                        .to_owned(),
                ));
            }
        }
        StageRole::Final => {
            if layer_range.end != num_layers {
                return Err(ComputeError::Message(
                    "final stage must end at num_layers".to_owned(),
                ));
            }
            if layer_range.start == 0 {
                return Err(ComputeError::Message(
                    "final stage cannot start at layer 0 in multi-stage mode".to_owned(),
                ));
            }
        }
        StageRole::Middle => {
            if layer_range.start == 0 || layer_range.end >= num_layers {
                return Err(ComputeError::Message(
                    "middle stage must be strictly interior".to_owned(),
                ));
            }
        }
    }
    Ok(())
}
