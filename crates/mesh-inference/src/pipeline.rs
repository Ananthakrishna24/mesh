use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

use mesh_compute::{group_complete_weight_files, Qwen3Stage, WeightFile};
use mesh_core::{
    stage_kv_reserve_bytes, ActivationHeader, DeploymentId, LayerRange, PlacementPlan, RequestId,
    SamplingParams, StageRole, StopReason, TokenResultEvent, TransferKind, FIRST_CONTEXT_LIMIT,
    FIRST_MAX_CONCURRENT_REQUESTS, ACTIVATION_MAX_IN_FLIGHT_PER_STAGE_REQUEST,
};
use mesh_model::{PrepareResult, ResolvedModel};
use thiserror::Error;

use crate::engine::locate_sidecar;
use crate::sampler::Sampler;
use crate::tokenizer::MeshTokenizer;
use crate::{EngineError, GenerationOutput};

const QWEN3_EOS_TOKEN_ID: u32 = 151_645;
const ALLOCATOR_OVERHEAD_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum PipelineError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Engine(#[from] EngineError),
    #[error(transparent)]
    Compute(#[from] mesh_compute::ComputeError),
}

#[derive(Debug, Clone)]
struct BoundedQueue<T> {
    items: VecDeque<T>,
    capacity: usize,
}

impl<T> BoundedQueue<T> {
    fn new(capacity: usize) -> Self {
        Self {
            items: VecDeque::new(),
            capacity: capacity.max(1),
        }
    }

    fn push(&mut self, item: T) -> Result<(), PipelineError> {
        if self.items.len() >= self.capacity {
            return Err(PipelineError::Message(
                "activation queue full".to_owned(),
            ));
        }
        self.items.push_back(item);
        Ok(())
    }

    fn pop(&mut self) -> Option<T> {
        self.items.pop_front()
    }

    fn clear(&mut self) {
        self.items.clear();
    }

    fn len(&self) -> usize {
        self.items.len()
    }
}

#[derive(Debug, Clone)]
struct PendingActivation {
    header: ActivationHeader,
    payload: Vec<u8>,
}

pub struct StageWorker {
    pub stage_index: u16,
    pub role: StageRole,
    pub layer_range: LayerRange,
    stage: Qwen3Stage,
    inbound: BoundedQueue<PendingActivation>,
    cancelled: HashSet<RequestId>,
    next_transfer_id: HashMap<RequestId, u64>,
    active_requests: HashSet<RequestId>,
}

impl StageWorker {
    pub fn load(
        stage_index: u16,
        role: StageRole,
        layer_range: LayerRange,
        config_path: &Path,
        weight_files: &[WeightFile],
        prefer_cuda: bool,
    ) -> Result<Self, PipelineError> {
        let stage = Qwen3Stage::load(config_path, weight_files, role, layer_range, prefer_cuda)?;
        Ok(Self {
            stage_index,
            role,
            layer_range,
            stage,
            inbound: BoundedQueue::new(ACTIVATION_MAX_IN_FLIGHT_PER_STAGE_REQUEST as usize),
            cancelled: HashSet::new(),
            next_transfer_id: HashMap::new(),
            active_requests: HashSet::new(),
        })
    }

    pub fn backend(&self) -> mesh_compute::BackendKind {
        self.stage.backend
    }

    pub fn reservation_memory_bytes(&self, weight_bytes: u64) -> u64 {
        weight_bytes.saturating_add(stage_kv_reserve_bytes(
            1,
            self.stage.num_kv_heads(),
            FIRST_CONTEXT_LIMIT,
            self.stage.head_dim(),
            self.stage.num_layers_owned(),
            FIRST_MAX_CONCURRENT_REQUESTS,
            ALLOCATOR_OVERHEAD_BYTES,
        ))
    }

    pub fn cancel(&mut self, request_id: RequestId) {
        self.cancelled.insert(request_id);
        self.active_requests.remove(&request_id);
        self.next_transfer_id.remove(&request_id);
        self.stage.clear_kv_cache();
        self.inbound.clear();
    }

    pub fn is_cancelled(&self, request_id: RequestId) -> bool {
        self.cancelled.contains(&request_id)
    }

    pub fn queue_depth(&self) -> usize {
        self.inbound.len()
    }

    fn allocate_transfer_id(&mut self, request_id: RequestId) -> u64 {
        let entry = self.next_transfer_id.entry(request_id).or_insert(1);
        let id = *entry;
        *entry = entry.saturating_add(1);
        id
    }

    fn encode_outgoing(
        &mut self,
        deployment_id: DeploymentId,
        request_id: RequestId,
        transfer_kind: TransferKind,
        sequence_position: u64,
        activation: &candle_core::Tensor,
    ) -> Result<PendingActivation, PipelineError> {
        let (batch, sequence, hidden) = activation.dims3().map_err(mesh_compute::ComputeError::from)?;
        let transfer_id = self.allocate_transfer_id(request_id);
        let header = ActivationHeader::qwen3_hidden(
            deployment_id,
            request_id,
            transfer_id,
            self.stage_index,
            self.stage_index.saturating_add(1),
            transfer_kind,
            batch as u64,
            sequence as u64,
            hidden as u64,
            sequence_position,
        )
        .map_err(|error| PipelineError::Message(error.to_string()))?;
        let payload = self
            .stage
            .activation_to_fp16_bytes(activation)
            .map_err(PipelineError::from)?;
        Ok(PendingActivation { header, payload })
    }

    fn decode_incoming(
        &self,
        pending: &PendingActivation,
    ) -> Result<candle_core::Tensor, PipelineError> {
        let dims = pending.header.used_dimensions();
        if dims.len() != 3 {
            return Err(PipelineError::Message(
                "pipeline activations must be rank 3".to_owned(),
            ));
        }
        self.stage
            .activation_from_fp16_bytes(
                &pending.payload,
                dims[0] as usize,
                dims[1] as usize,
                dims[2] as usize,
            )
            .map_err(PipelineError::from)
    }
}

pub struct PipelineEngine {
    pub deployment_id: DeploymentId,
    pub model_line: String,
    pub placement: PlacementPlan,
    pub stages: Vec<StageWorker>,
    tokenizer: MeshTokenizer,
}

impl PipelineEngine {
    pub fn load_in_process(
        deployment_id: DeploymentId,
        resolved: &ResolvedModel,
        prepared: &PrepareResult,
        cache_root: &Path,
        placement: PlacementPlan,
        prefer_cuda: bool,
        hf_tokenizer_path: Option<&Path>,
    ) -> Result<Self, PipelineError> {
        placement
            .validate()
            .map_err(PipelineError::Message)?;
        if placement.deployment_id != deployment_id {
            return Err(PipelineError::Message(
                "placement deployment_id mismatch".to_owned(),
            ));
        }

        let prepared_files = prepared
            .prepared
            .iter()
            .map(|item| {
                (
                    item.artifact_path.clone(),
                    item.relative_path.clone(),
                    item.range_start,
                    item.range_end,
                )
            })
            .collect::<Vec<_>>();
        let weight_files = group_complete_weight_files(cache_root, &prepared_files)?;
        let config_path = locate_sidecar(
            cache_root,
            hf_tokenizer_path.and_then(Path::parent),
            &resolved.identity.repository,
            &resolved.identity.revision,
            "config.json",
        )
        .map_err(PipelineError::from)?;
        let tokenizer_path = locate_sidecar(
            cache_root,
            hf_tokenizer_path.and_then(Path::parent),
            &resolved.identity.repository,
            &resolved.identity.revision,
            "tokenizer.json",
        )
        .map_err(PipelineError::from)?;
        let tokenizer = MeshTokenizer::load(
            &tokenizer_path,
            &resolved.identity.tokenizer_hash,
            QWEN3_EOS_TOKEN_ID,
        )
        .map_err(EngineError::from)?;

        let mut stages = Vec::with_capacity(placement.stages.len());
        for assignment in &placement.stages {
            stages.push(StageWorker::load(
                assignment.stage_index,
                assignment.role,
                assignment.layer_range,
                &config_path,
                &weight_files,
                prefer_cuda,
            )?);
        }

        Ok(Self {
            deployment_id,
            model_line: resolved.identity.summary_line(),
            placement,
            stages,
            tokenizer,
        })
    }

    pub fn tokenizer(&self) -> &MeshTokenizer {
        &self.tokenizer
    }

    pub fn cancel(&mut self, request_id: RequestId) {
        for stage in &mut self.stages {
            stage.cancel(request_id);
        }
    }

    pub fn generate_from_tokens(
        &mut self,
        prompt_token_ids: &[u32],
        params: SamplingParams,
        stop_token_ids: &[u32],
        request_id: RequestId,
        mut on_token: impl FnMut(&TokenResultEvent),
        mut should_continue: impl FnMut() -> bool,
    ) -> Result<GenerationOutput, PipelineError> {
        if prompt_token_ids.is_empty() {
            return Err(PipelineError::Message(
                "prompt must contain tokens".to_owned(),
            ));
        }
        if self.stages.is_empty() {
            return Err(PipelineError::Message(
                "pipeline has no stages".to_owned(),
            ));
        }

        for stage in &mut self.stages {
            stage.cancelled.remove(&request_id);
            stage.active_requests.insert(request_id);
            stage.stage.clear_kv_cache();
            stage.inbound.clear();
            stage.next_transfer_id.insert(request_id, 1);
        }

        let final_index = self.stages.len() - 1;
        let vocab_size = self.stages[final_index].stage.vocab_size();
        let context_limit = self.stages[final_index].stage.context_limit();
        let mut sampler = Sampler::new(
            params,
            vocab_size,
            self.tokenizer.eos_token_id,
            stop_token_ids.to_vec(),
            context_limit,
            prompt_token_ids,
        )
        .map_err(PipelineError::Message)?;

        let mut events: Vec<TokenResultEvent> = Vec::new();
        let mut generated_ids = Vec::new();
        let stop_reason;

        let mut logits = self.run_prefill(request_id, prompt_token_ids)?;
        loop {
            if !should_continue() || self.stages.iter().any(|stage| stage.is_cancelled(request_id))
            {
                stop_reason = StopReason::Cancelled;
                if let Some(last) = events.last_mut() {
                    last.is_last = true;
                    last.stop_reason = Some(StopReason::Cancelled);
                    on_token(last);
                }
                break;
            }

            let outcome = sampler.sample(&logits).map_err(PipelineError::Message)?;
            let event = TokenResultEvent {
                deployment_id: self.deployment_id,
                request_id,
                token_id: outcome.token_id,
                token_index: outcome.token_index,
                is_last: outcome.is_last,
                stop_reason: outcome.stop_reason,
                sequence_length: outcome.sequence_length,
            };
            generated_ids.push(outcome.token_id);
            on_token(&event);
            events.push(event);
            if outcome.is_last {
                stop_reason = outcome.stop_reason.unwrap_or(StopReason::Error);
                break;
            }
            logits = self.run_decode(request_id, outcome.token_id)?;
        }

        for stage in &mut self.stages {
            stage.active_requests.remove(&request_id);
            stage.next_transfer_id.remove(&request_id);
            stage.stage.clear_kv_cache();
            stage.inbound.clear();
        }

        let text = self
            .tokenizer
            .decode_stream(&generated_ids)
            .map_err(EngineError::from)?;
        Ok(GenerationOutput {
            text,
            tokens: events,
            stop_reason,
        })
    }

    pub fn generate(
        &mut self,
        prompt: &str,
        params: SamplingParams,
        should_continue: impl FnMut() -> bool,
    ) -> Result<GenerationOutput, PipelineError> {
        let token_ids = self
            .tokenizer
            .encode_chat(None, prompt)
            .map_err(EngineError::from)?;
        self.generate_from_tokens(&token_ids, params, &[], RequestId::new(), |_| {}, should_continue)
    }

    fn run_prefill(
        &mut self,
        request_id: RequestId,
        prompt_token_ids: &[u32],
    ) -> Result<Vec<f32>, PipelineError> {
        let mut pending = {
            let first = self
                .stages
                .first_mut()
                .ok_or_else(|| PipelineError::Message("missing first stage".to_owned()))?;
            if first.is_cancelled(request_id) {
                return Err(PipelineError::Message("request cancelled".to_owned()));
            }
            let activation = first.stage.prefill_tokens(prompt_token_ids)?;
            if first.role.emits_logits() {
                return first.stage.logits_from_hidden(&activation).map_err(Into::into);
            }
            first.encode_outgoing(
                self.deployment_id,
                request_id,
                TransferKind::Prefill,
                0,
                &activation,
            )?
        };

        for index in 1..self.stages.len() {
            if self.stages[index].is_cancelled(request_id) {
                return Err(PipelineError::Message("request cancelled".to_owned()));
            }
            self.stages[index].inbound.push(pending)?;
            let queued = self.stages[index]
                .inbound
                .pop()
                .ok_or_else(|| PipelineError::Message("missing queued activation".to_owned()))?;
            let tensor = self.stages[index].decode_incoming(&queued)?;
            let activation = self.stages[index].stage.prefill_activation(&tensor)?;
            if index + 1 == self.stages.len() {
                return self.stages[index]
                    .stage
                    .logits_from_hidden(&activation)
                    .map_err(Into::into);
            }
            pending = self.stages[index].encode_outgoing(
                self.deployment_id,
                request_id,
                TransferKind::Prefill,
                queued.header.sequence_position,
                &activation,
            )?;
        }

        Err(PipelineError::Message(
            "pipeline prefill failed to reach final stage".to_owned(),
        ))
    }

    fn run_decode(
        &mut self,
        request_id: RequestId,
        token_id: u32,
    ) -> Result<Vec<f32>, PipelineError> {
        let sequence_position = {
            let first = self
                .stages
                .first()
                .ok_or_else(|| PipelineError::Message("missing first stage".to_owned()))?;
            first.stage.seq_len() as u64
        };

        let mut pending = {
            let first = self
                .stages
                .first_mut()
                .ok_or_else(|| PipelineError::Message("missing first stage".to_owned()))?;
            if first.is_cancelled(request_id) {
                return Err(PipelineError::Message("request cancelled".to_owned()));
            }
            let activation = first.stage.decode_token(token_id)?;
            if first.role.emits_logits() {
                return first.stage.logits_from_hidden(&activation).map_err(Into::into);
            }
            first.encode_outgoing(
                self.deployment_id,
                request_id,
                TransferKind::Decode,
                sequence_position,
                &activation,
            )?
        };

        for index in 1..self.stages.len() {
            if self.stages[index].is_cancelled(request_id) {
                return Err(PipelineError::Message("request cancelled".to_owned()));
            }
            self.stages[index].inbound.push(pending)?;
            let queued = self.stages[index]
                .inbound
                .pop()
                .ok_or_else(|| PipelineError::Message("missing queued activation".to_owned()))?;
            let tensor = self.stages[index].decode_incoming(&queued)?;
            let activation = self.stages[index].stage.decode_activation(&tensor)?;
            if index + 1 == self.stages.len() {
                return self.stages[index]
                    .stage
                    .logits_from_hidden(&activation)
                    .map_err(Into::into);
            }
            pending = self.stages[index].encode_outgoing(
                self.deployment_id,
                request_id,
                TransferKind::Decode,
                queued.header.sequence_position,
                &activation,
            )?;
        }

        Err(PipelineError::Message(
            "pipeline decode failed to reach final stage".to_owned(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesh_compute::{LoadedQwen3, WeightFile};
    use mesh_core::{DeploymentId, NodeId, PlacementPlan, RequestId, SamplingParams, StageRole};
    use std::path::PathBuf;

    fn smoke_root() -> Option<PathBuf> {
        if std::env::var_os("MESH_P09_SMOKE").is_none() && std::env::var_os("MESH_P07_SMOKE").is_none()
        {
            return None;
        }
        let root = std::env::var_os("MESH_P07_DATA_DIR")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join("mesh-p07-smoke")))?;
        if root.join("model-cache").is_dir() {
            Some(root)
        } else {
            None
        }
    }

    fn weight_files(root: &Path) -> Vec<WeightFile> {
        let mut files = Vec::new();
        let objects = root
            .join("model-cache/objects/huggingface");
        for path in walkdir_files(&objects) {
            files.push(WeightFile {
                artifact_path: path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "weight".to_owned()),
                absolute_path: path,
            });
        }
        files.sort_by(|a, b| a.absolute_path.cmp(&b.absolute_path));
        files
    }

    fn walkdir_files(root: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.is_file() {
                    out.push(path);
                }
            }
        }
        out
    }

    fn config_path(root: &Path) -> PathBuf {
        let snapshots = root.join("cache/hf-hub/models--Qwen--Qwen3-4B/snapshots");
        for path in walkdir_files(&snapshots) {
            if path.file_name().and_then(|name| name.to_str()) == Some("config.json") {
                return path;
            }
        }
        panic!("config.json missing under {}", snapshots.display());
    }

    fn tokenizer_path(root: &Path) -> PathBuf {
        let snapshots = root.join("cache/hf-hub/models--Qwen--Qwen3-4B/snapshots");
        for path in walkdir_files(&snapshots) {
            if path.file_name().and_then(|name| name.to_str()) == Some("tokenizer.json") {
                return path;
            }
        }
        panic!("tokenizer.json missing under {}", snapshots.display());
    }

    #[test]
    fn p09_two_stage_matches_complete_greedy() {
        let Some(root) = smoke_root() else {
            eprintln!("skipping P09 two-stage smoke; set MESH_P09_SMOKE=1 and MESH_P07_DATA_DIR");
            return;
        };
        let prefer_cuda = cfg!(feature = "cuda");
        let weights = weight_files(&root);
        assert!(
            weights.len() >= 2,
            "expected whole-shard weight files in {}",
            root.display()
        );
        let config = config_path(&root);
        let tokenizer = MeshTokenizer::load(&tokenizer_path(&root), "", QWEN3_EOS_TOKEN_ID)
            .expect("tokenizer");
        let prompt_tokens = tokenizer.encode_chat(None, "Say hi").expect("encode");
        let params = SamplingParams {
            temperature: 0.0,
            top_k: 0,
            top_p: 1.0,
            repetition_penalty: 1.0,
            seed: 7,
            max_new_tokens: 4,
        };

        let complete_tokens = {
            let mut complete = LoadedQwen3::load(&config, &weights, prefer_cuda).expect("complete");
            let mut sampler = Sampler::new(
                params,
                complete.vocab_size(),
                tokenizer.eos_token_id,
                Vec::new(),
                complete.context_limit(),
                &prompt_tokens,
            )
            .expect("sampler");
            let mut logits = complete.prefill_logits(&prompt_tokens).expect("prefill");
            let mut ids = Vec::new();
            loop {
                let outcome = sampler.sample(&logits).expect("sample");
                ids.push(outcome.token_id);
                if outcome.is_last {
                    break;
                }
                logits = complete.decode_logits(outcome.token_id).expect("decode");
            }
            complete.clear_kv_cache();
            drop(complete);
            ids
        };

        let deployment_id = DeploymentId::from_bytes([9; 16]);
        let nodes = [
            NodeId::from_bytes([1; 32]),
            NodeId::from_bytes([2; 32]),
        ];
        let placement = PlacementPlan::split_even(deployment_id, "Qwen/Qwen3-4B", 36, &nodes)
            .expect("placement");
        let mut stages = Vec::new();
        for assignment in &placement.stages {
            stages.push(
                StageWorker::load(
                    assignment.stage_index,
                    assignment.role,
                    assignment.layer_range,
                    &config,
                    &weights,
                    prefer_cuda,
                )
                .expect("stage load"),
            );
        }
        let mut pipeline = PipelineEngine {
            deployment_id,
            model_line: "Qwen/Qwen3-4B".to_owned(),
            placement,
            stages,
            tokenizer,
        };
        let output = pipeline
            .generate_from_tokens(
                &prompt_tokens,
                params,
                &[],
                RequestId::from_bytes([3; 16]),
                |_| {},
                || true,
            )
            .expect("pipeline generate");
        let pipeline_tokens = output
            .tokens
            .iter()
            .map(|token| token.token_id)
            .collect::<Vec<_>>();

        assert_eq!(
            pipeline_tokens, complete_tokens,
            "two-stage tokens {:?} != complete {:?}",
            pipeline_tokens, complete_tokens
        );
        assert!(matches!(
            pipeline.stages[0].role,
            StageRole::First
        ));
        assert!(matches!(
            pipeline.stages[1].role,
            StageRole::Final
        ));
        eprintln!(
            "P09 two-stage match ok backend={} tokens={:?} text={:?}",
            pipeline.stages[0].backend().as_str(),
            pipeline_tokens,
            output.text
        );
    }

    #[test]
    fn bounded_queue_rejects_over_capacity() {
        let mut queue = BoundedQueue::new(2);
        queue.push(1).unwrap();
        queue.push(2).unwrap();
        assert!(queue.push(3).unwrap_err().to_string().contains("queue full"));
        assert_eq!(queue.pop(), Some(1));
    }
}


