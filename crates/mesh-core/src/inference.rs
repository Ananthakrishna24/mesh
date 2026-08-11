use serde::{Deserialize, Serialize};

use crate::{DeploymentId, NodeId, RequestId};

pub const FIRST_CONTEXT_LIMIT: u32 = 4096;
pub const FIRST_MAX_CONCURRENT_REQUESTS: u32 = 1;
pub const PIPELINE_MAX_CONCURRENT_REQUESTS: u32 = 2;
pub const DEFAULT_TEMPERATURE: f32 = 0.7;
pub const DEFAULT_TOP_P: f32 = 0.8;
pub const DEFAULT_TOP_K: u32 = 20;
pub const DEFAULT_REPETITION_PENALTY: f32 = 1.0;
pub const DEFAULT_MAX_NEW_TOKENS: u32 = 128;
pub const WARMUP_MAX_NEW_TOKENS: u32 = 8;
pub const KV_BYTES_PER_ELEMENT: u64 = 2;
pub const MAX_PIPELINE_STAGES: usize = 3;

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

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "eos" => Some(Self::Eos),
            "max_new_tokens" => Some(Self::MaxNewTokens),
            "context_limit" => Some(Self::ContextLimit),
            "cancelled" => Some(Self::Cancelled),
            "error" => Some(Self::Error),
            _ => None,
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

    pub fn validate(
        &self,
        prompt_len: u32,
        context_limit: u32,
        vocab_size: u32,
    ) -> Result<(), String> {
        if !(0.0..=2.0).contains(&self.temperature) {
            return Err(format!(
                "temperature {} out of range 0..=2",
                self.temperature
            ));
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NextTokenFeedback {
    pub deployment_id: DeploymentId,
    pub request_id: RequestId,
    pub token_id: u32,
    pub token_index: u32,
    pub is_last: bool,
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
    pub routed_node_id: Option<String>,
    pub replicas: Vec<ReplicaEndpointView>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StageRole {
    First,
    Middle,
    Final,
    Complete,
}

impl StageRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::First => "first",
            Self::Middle => "middle",
            Self::Final => "final",
            Self::Complete => "complete",
        }
    }

    pub fn owns_embeddings(self) -> bool {
        matches!(self, Self::First | Self::Complete)
    }

    pub fn owns_output_head(self) -> bool {
        matches!(self, Self::Final | Self::Complete)
    }

    pub fn accepts_token_ids(self) -> bool {
        self.owns_embeddings()
    }

    pub fn emits_logits(self) -> bool {
        self.owns_output_head()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayerRange {
    pub start: u32,
    pub end: u32,
}

impl LayerRange {
    pub fn new(start: u32, end: u32) -> Result<Self, String> {
        if start >= end {
            return Err(format!("layer range start {start} must be < end {end}"));
        }
        Ok(Self { start, end })
    }

    pub fn len(self) -> u32 {
        self.end.saturating_sub(self.start)
    }

    pub fn contains(self, layer: u32) -> bool {
        layer >= self.start && layer < self.end
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageAssignment {
    pub stage_index: u16,
    pub role: StageRole,
    pub node_id: NodeId,
    pub layer_range: LayerRange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlacementPlan {
    pub deployment_id: DeploymentId,
    pub model_line: String,
    pub num_layers: u32,
    pub stages: Vec<StageAssignment>,
}

impl PlacementPlan {
    pub fn validate(&self) -> Result<(), String> {
        if self.stages.is_empty() {
            return Err("placement requires at least one stage".to_owned());
        }
        if self.stages.len() > MAX_PIPELINE_STAGES {
            return Err(format!(
                "placement has {} stages; max is {MAX_PIPELINE_STAGES}",
                self.stages.len()
            ));
        }
        if self.num_layers == 0 {
            return Err("placement num_layers must be > 0".to_owned());
        }

        let mut expected_start = 0u32;
        for (index, stage) in self.stages.iter().enumerate() {
            if stage.stage_index as usize != index {
                return Err(format!(
                    "stage_index {} must equal position {index}",
                    stage.stage_index
                ));
            }
            if stage.layer_range.start != expected_start {
                return Err(format!(
                    "stage {index} must start at layer {expected_start}, got {}",
                    stage.layer_range.start
                ));
            }
            if stage.layer_range.end > self.num_layers {
                return Err(format!(
                    "stage {index} end {} exceeds num_layers {}",
                    stage.layer_range.end, self.num_layers
                ));
            }
            expected_start = stage.layer_range.end;

            let expected_role = if self.stages.len() == 1 {
                StageRole::Complete
            } else if index == 0 {
                StageRole::First
            } else if index + 1 == self.stages.len() {
                StageRole::Final
            } else {
                StageRole::Middle
            };
            if stage.role != expected_role {
                return Err(format!(
                    "stage {index} role {:?} does not match expected {:?}",
                    stage.role, expected_role
                ));
            }
        }
        if expected_start != self.num_layers {
            return Err(format!(
                "layers covered through {expected_start}, expected {}",
                self.num_layers
            ));
        }
        Ok(())
    }

    pub fn split_even(
        deployment_id: DeploymentId,
        model_line: impl Into<String>,
        num_layers: u32,
        node_ids: &[NodeId],
    ) -> Result<Self, String> {
        if node_ids.is_empty() {
            return Err("split_even requires at least one node".to_owned());
        }
        if node_ids.len() > MAX_PIPELINE_STAGES {
            return Err(format!(
                "split_even supports at most {MAX_PIPELINE_STAGES} stages"
            ));
        }
        if num_layers < node_ids.len() as u32 {
            return Err("num_layers must be >= stage count".to_owned());
        }
        let stage_count = node_ids.len() as u32;
        let base = num_layers / stage_count;
        let rem = num_layers % stage_count;
        let mut stages = Vec::with_capacity(node_ids.len());
        let mut start = 0u32;
        for (index, node_id) in node_ids.iter().enumerate() {
            let extra = if (index as u32) < rem { 1 } else { 0 };
            let end = start + base + extra;
            let role = if node_ids.len() == 1 {
                StageRole::Complete
            } else if index == 0 {
                StageRole::First
            } else if index + 1 == node_ids.len() {
                StageRole::Final
            } else {
                StageRole::Middle
            };
            stages.push(StageAssignment {
                stage_index: index as u16,
                role,
                node_id: *node_id,
                layer_range: LayerRange::new(start, end)?,
            });
            start = end;
        }
        let plan = Self {
            deployment_id,
            model_line: model_line.into(),
            num_layers,
            stages,
        };
        plan.validate()?;
        Ok(plan)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReplicaHealth {
    Ready,
    Busy,
    Unhealthy,
    Offline,
}

impl ReplicaHealth {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Busy => "busy",
            Self::Unhealthy => "unhealthy",
            Self::Offline => "offline",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplicaEndpointView {
    pub node_id: String,
    pub display_name: String,
    pub deployment_id: String,
    pub model_line: String,
    pub backend: String,
    pub ready: bool,
    pub healthy: bool,
    pub active_requests: u32,
    pub max_concurrent_requests: u32,
    pub health: ReplicaHealth,
    pub local: bool,
}

impl ReplicaEndpointView {
    pub fn free_slots(&self) -> u32 {
        self.max_concurrent_requests
            .saturating_sub(self.active_requests)
    }

    pub fn can_accept(&self) -> bool {
        self.ready && self.healthy && self.free_slots() > 0
    }

    pub fn status_line(&self) -> String {
        format!(
            "{} · {} · {}/{}/{} · {}",
            self.display_name,
            self.backend,
            self.active_requests,
            self.max_concurrent_requests,
            if self.local { "local" } else { "remote" },
            self.health.as_str()
        )
    }
}

pub fn select_replica_route<'a>(
    replicas: impl IntoIterator<Item = &'a ReplicaEndpointView>,
    model_line: Option<&str>,
) -> Option<&'a ReplicaEndpointView> {
    let mut best: Option<&ReplicaEndpointView> = None;
    for replica in replicas {
        if !replica.can_accept() {
            continue;
        }
        if let Some(model_line) = model_line {
            if replica.model_line != model_line {
                continue;
            }
        }
        best = Some(match best {
            None => replica,
            Some(current) => {
                let left = (
                    current.active_requests,
                    u8::from(current.local),
                    current.node_id.as_str(),
                );
                let right = (
                    replica.active_requests,
                    u8::from(replica.local),
                    replica.node_id.as_str(),
                );
                if right < left { replica } else { current }
            }
        });
    }
    best
}

pub fn per_layer_kv_bytes(batch: u32, num_kv_heads: u32, seq_capacity: u32, head_dim: u32) -> u64 {
    2u64.saturating_mul(u64::from(batch))
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

    #[test]
    fn select_replica_route_prefers_remote_when_load_is_equal() {
        let local_busy = ReplicaEndpointView {
            node_id: "aa".into(),
            display_name: "Local".into(),
            deployment_id: "d1".into(),
            model_line: "Qwen/Qwen3-4B".into(),
            backend: "cuda".into(),
            ready: true,
            healthy: true,
            active_requests: 1,
            max_concurrent_requests: 1,
            health: ReplicaHealth::Busy,
            local: true,
        };
        let remote_ready = ReplicaEndpointView {
            node_id: "bb".into(),
            display_name: "Remote".into(),
            deployment_id: "d2".into(),
            model_line: "Qwen/Qwen3-4B".into(),
            backend: "cuda".into(),
            ready: true,
            healthy: true,
            active_requests: 0,
            max_concurrent_requests: 1,
            health: ReplicaHealth::Ready,
            local: false,
        };
        let local_ready = ReplicaEndpointView {
            node_id: "cc".into(),
            display_name: "LocalReady".into(),
            deployment_id: "d3".into(),
            model_line: "Qwen/Qwen3-4B".into(),
            backend: "cpu".into(),
            ready: true,
            healthy: true,
            active_requests: 0,
            max_concurrent_requests: 1,
            health: ReplicaHealth::Ready,
            local: true,
        };
        let chosen = select_replica_route(
            [&local_busy, &remote_ready, &local_ready],
            Some("Qwen/Qwen3-4B"),
        )
        .expect("route");
        assert_eq!(chosen.node_id, "bb");
        assert!(!chosen.local);
    }

    #[test]
    fn placement_split_even_two_stages() {
        let a = NodeId::from_bytes([1; 32]);
        let b = NodeId::from_bytes([2; 32]);
        let plan = PlacementPlan::split_even(
            DeploymentId::from_bytes([9; 16]),
            "Qwen/Qwen3-4B",
            36,
            &[a, b],
        )
        .expect("plan");
        assert_eq!(plan.stages.len(), 2);
        assert_eq!(plan.stages[0].role, StageRole::First);
        assert_eq!(plan.stages[0].layer_range, LayerRange { start: 0, end: 18 });
        assert_eq!(plan.stages[1].role, StageRole::Final);
        assert_eq!(
            plan.stages[1].layer_range,
            LayerRange { start: 18, end: 36 }
        );
        plan.validate().unwrap();
    }

    #[test]
    fn placement_rejects_gap() {
        let plan = PlacementPlan {
            deployment_id: DeploymentId::from_bytes([1; 16]),
            model_line: "x".into(),
            num_layers: 4,
            stages: vec![
                StageAssignment {
                    stage_index: 0,
                    role: StageRole::First,
                    node_id: NodeId::from_bytes([1; 32]),
                    layer_range: LayerRange { start: 0, end: 1 },
                },
                StageAssignment {
                    stage_index: 1,
                    role: StageRole::Final,
                    node_id: NodeId::from_bytes([2; 32]),
                    layer_range: LayerRange { start: 2, end: 4 },
                },
            ],
        };
        assert!(
            plan.validate()
                .unwrap_err()
                .contains("must start at layer 1")
        );
    }
}
