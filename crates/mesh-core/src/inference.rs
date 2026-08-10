use serde::{Deserialize, Serialize};

use crate::{DeploymentId, RequestId};

pub const FIRST_CONTEXT_LIMIT: u32 = 4096;
pub const FIRST_MAX_CONCURRENT_REQUESTS: u32 = 1;
pub const DEFAULT_TEMPERATURE: f32 = 0.7;
pub const DEFAULT_TOP_P: f32 = 0.8;
pub const DEFAULT_TOP_K: u32 = 20;
pub const DEFAULT_REPETITION_PENALTY: f32 = 1.0;
pub const DEFAULT_MAX_NEW_TOKENS: u32 = 128;
pub const WARMUP_MAX_NEW_TOKENS: u32 = 8;
pub const KV_BYTES_PER_ELEMENT: u64 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StopReason {
    Eos,
    MaxNewTokens,
    ContextLimit,
    Cancelled,
    Error,
}

impl StopReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Eos => "eos",
            Self::MaxNewTokens => "max_new_tokens",
            Self::ContextLimit => "context_limit",
            Self::Cancelled => "cancelled",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InferencePhase {
    Idle,
    Loading,
    WarmingUp,
    Ready,
    Generating,
    Failed,
}

impl InferencePhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Loading => "loading",
            Self::WarmingUp => "warming_up",
            Self::Ready => "ready",
            Self::Generating => "generating",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SamplingParams {
    pub temperature: f32,
    pub top_k: u32,
    pub top_p: f32,
    pub repetition_penalty: f32,
    pub seed: u64,
    pub max_new_tokens: u32,
}

impl SamplingParams {
    pub fn non_thinking_default(seed: u64) -> Self {
        Self {
            temperature: DEFAULT_TEMPERATURE,
            top_k: DEFAULT_TOP_K,
            top_p: DEFAULT_TOP_P,
            repetition_penalty: DEFAULT_REPETITION_PENALTY,
            seed,
            max_new_tokens: DEFAULT_MAX_NEW_TOKENS,
        }
    }

    pub fn warmup(seed: u64) -> Self {
        Self {
            temperature: 0.0,
            top_k: 0,
            top_p: 1.0,
            repetition_penalty: 1.0,
            seed,
            max_new_tokens: WARMUP_MAX_NEW_TOKENS,
        }
    }

    pub fn validate(&self, prompt_len: u32, context_limit: u32, vocab_size: u32) -> Result<(), String> {
        if !(0.0..=2.0).contains(&self.temperature) {
            return Err(format!("temperature {} out of range 0..=2", self.temperature));
        }
        if self.top_k != 0 && !(1..=vocab_size).contains(&self.top_k) {
            return Err(format!("top_k {} out of range", self.top_k));
        }
        if !(0.0..=1.0).contains(&self.top_p) {
            return Err(format!("top_p {} out of range 0..=1", self.top_p));
        }
        if !(0.1..=2.0).contains(&self.repetition_penalty) {
            return Err(format!(
                "repetition_penalty {} out of range 0.1..=2",
                self.repetition_penalty
            ));
        }
        if self.max_new_tokens == 0 {
            return Err("max_new_tokens must be >= 1".to_owned());
        }
        if prompt_len == 0 {
            return Err("prompt must encode to at least one token".to_owned());
        }
        if prompt_len.saturating_add(self.max_new_tokens) > context_limit {
            return Err(format!(
                "prompt ({prompt_len}) + max_new_tokens ({}) exceeds context limit {context_limit}",
                self.max_new_tokens
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InferenceRequestSpec {
    pub deployment_id: DeploymentId,
    pub request_id: RequestId,
    pub input_token_ids: Vec<u32>,
    pub sampling: SamplingParams,
    pub stop_token_ids: Vec<u32>,
    pub return_logprobs: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenResultEvent {
    pub deployment_id: DeploymentId,
    pub request_id: RequestId,
    pub token_id: u32,
    pub token_index: u32,
    pub is_last: bool,
    pub stop_reason: Option<StopReason>,
    pub sequence_length: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct InferenceView {
    pub phase: Option<InferencePhase>,
    pub deployment_id: Option<String>,
    pub model_line: Option<String>,
    pub backend: Option<String>,
    pub status_line: String,
    pub error: Option<String>,
    pub busy: bool,
    pub prompt: String,
    pub output_text: String,
    pub generated_tokens: u32,
    pub stop_reason: Option<String>,
    pub last_token_id: Option<u32>,
}

impl InferenceView {
    pub fn idle() -> Self {
        Self {
            phase: Some(InferencePhase::Idle),
            status_line: "Inference idle".to_owned(),
            ..Self::default()
        }
    }
}

pub fn per_layer_kv_bytes(
    batch: u32,
    num_kv_heads: u32,
    seq_capacity: u32,
    head_dim: u32,
) -> u64 {
    2u64
        .saturating_mul(u64::from(batch))
        .saturating_mul(u64::from(num_kv_heads))
        .saturating_mul(u64::from(seq_capacity))
        .saturating_mul(u64::from(head_dim))
        .saturating_mul(KV_BYTES_PER_ELEMENT)
}

pub fn request_stage_kv_bytes(
    batch: u32,
    num_kv_heads: u32,
    seq_capacity: u32,
    head_dim: u32,
    layer_count: u32,
) -> u64 {
    per_layer_kv_bytes(batch, num_kv_heads, seq_capacity, head_dim)
        .saturating_mul(u64::from(layer_count))
}

pub fn stage_kv_reserve_bytes(
    batch: u32,
    num_kv_heads: u32,
    seq_capacity: u32,
    head_dim: u32,
    layer_count: u32,
    max_concurrent_requests: u32,
    allocator_overhead_bytes: u64,
) -> u64 {
    request_stage_kv_bytes(batch, num_kv_heads, seq_capacity, head_dim, layer_count)
        .saturating_mul(u64::from(max_concurrent_requests))
        .saturating_add(allocator_overhead_bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qwen3_kv_layer_is_16_mib_at_4k() {
        let bytes = per_layer_kv_bytes(1, 8, 4096, 128);
        assert_eq!(bytes, 16 * 1024 * 1024);
        assert_eq!(
            request_stage_kv_bytes(1, 8, 4096, 128, 36),
            576 * 1024 * 1024
        );
    }

    #[test]
    fn sampling_params_reject_context_overflow() {
        let params = SamplingParams {
            temperature: 0.0,
            top_k: 0,
            top_p: 1.0,
            repetition_penalty: 1.0,
            seed: 1,
            max_new_tokens: 100,
        };
        assert!(params.validate(4000, 4096, 151936).is_err());
        assert!(params.validate(10, 4096, 151936).is_ok());
    }
}
