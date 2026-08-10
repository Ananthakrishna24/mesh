use std::path::{Path, PathBuf};

use mesh_compute::{group_complete_weight_files, BackendKind, LoadedQwen3};
use mesh_core::{
    stage_kv_reserve_bytes, DeploymentId, FIRST_CONTEXT_LIMIT, FIRST_MAX_CONCURRENT_REQUESTS,
    InferencePhase, InferenceView, RequestId, SamplingParams, StopReason, TokenResultEvent,
};
use mesh_model::{PrepareResult, ResolvedModel};
use thiserror::Error;

use crate::sampler::Sampler;
use crate::tokenizer::MeshTokenizer;

const QWEN3_EOS_TOKEN_ID: u32 = 151_645;
const ALLOCATOR_OVERHEAD_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Compute(#[from] mesh_compute::ComputeError),
    #[error(transparent)]
    Tokenizer(#[from] crate::tokenizer::TokenizerError),
}

pub struct SingleNodeEngine {
    pub deployment_id: DeploymentId,
    pub model_line: String,
    pub backend: BackendKind,
    pub phase: InferencePhase,
    tokenizer: MeshTokenizer,
    model: LoadedQwen3,
    #[allow(dead_code)]
    config_path: PathBuf,
    #[allow(dead_code)]
    weight_files: Vec<mesh_compute::WeightFile>,
    reservation_memory_bytes: u64,
}

pub struct GenerationOutput {
    pub text: String,
    pub tokens: Vec<TokenResultEvent>,
    pub stop_reason: StopReason,
}

impl SingleNodeEngine {
    pub fn load(
        deployment_id: DeploymentId,
        resolved: &ResolvedModel,
        prepared: &PrepareResult,
        cache_root: &Path,
        prefer_cuda: bool,
        hf_tokenizer_path: Option<&Path>,
    ) -> Result<Self, EngineError> {
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
            hf_tokenizer_path.and_then(|path| path.parent()),
            &resolved.identity.repository,
            &resolved.identity.revision,
            "config.json",
        )?;
        let tokenizer_path = locate_sidecar(
            cache_root,
            hf_tokenizer_path.and_then(|path| path.parent()),
            &resolved.identity.repository,
            &resolved.identity.revision,
            "tokenizer.json",
        )?;

        let tokenizer = MeshTokenizer::load(
            &tokenizer_path,
            &resolved.identity.tokenizer_hash,
            QWEN3_EOS_TOKEN_ID,
        )?;
        let model = LoadedQwen3::load(&config_path, &weight_files, prefer_cuda)?;
        let reservation_memory_bytes = prepared.plan.gpu_bytes_reserved.saturating_add(
            stage_kv_reserve_bytes(
                1,
                model.num_kv_heads(),
                FIRST_CONTEXT_LIMIT,
                model.head_dim(),
                model.num_layers(),
                FIRST_MAX_CONCURRENT_REQUESTS,
                ALLOCATOR_OVERHEAD_BYTES,
            ),
        );

        Ok(Self {
            deployment_id,
            model_line: resolved.identity.summary_line(),
            backend: model.backend,
            phase: InferencePhase::Ready,
            tokenizer,
            model,
            config_path,
            weight_files,
            reservation_memory_bytes,
        })
    }

    pub fn reservation_memory_bytes(&self) -> u64 {
        self.reservation_memory_bytes
    }

    pub fn view(&self, prompt: &str, output: &str, error: Option<String>) -> InferenceView {
        InferenceView {
            phase: Some(self.phase),
            deployment_id: Some(self.deployment_id.to_string()),
            model_line: Some(self.model_line.clone()),
            backend: Some(self.backend.as_str().to_owned()),
            status_line: match self.phase {
                InferencePhase::Ready => format!(
                    "Ready on {} · {}",
                    self.backend.as_str(),
                    self.model_line
                ),
                InferencePhase::Generating => "Generating…".to_owned(),
                InferencePhase::Failed => "Inference failed".to_owned(),
                other => other.as_str().to_owned(),
            },
            error,
            busy: matches!(
                self.phase,
                InferencePhase::Loading | InferencePhase::WarmingUp | InferencePhase::Generating
            ),
            prompt: prompt.to_owned(),
            output_text: output.to_owned(),
            generated_tokens: 0,
            stop_reason: None,
            last_token_id: None,
        }
    }

    pub fn warmup(&mut self) -> Result<(), EngineError> {
        self.phase = InferencePhase::WarmingUp;
        let tokens = self
            .tokenizer
            .encode_chat(None, "ping")
            .map_err(EngineError::from)?;
        let params = SamplingParams::warmup(0);
        let _ = self.generate_tokens(&tokens, params, &[], || true)?;
        self.model.clear_kv_cache();
        self.phase = InferencePhase::Ready;
        Ok(())
    }

    pub fn generate(
        &mut self,
        prompt: &str,
        params: SamplingParams,
        should_continue: impl FnMut() -> bool,
    ) -> Result<GenerationOutput, EngineError> {
        self.phase = InferencePhase::Generating;
        let token_ids = self
            .tokenizer
            .encode_chat(None, prompt)
            .map_err(EngineError::from)?;
        let output = self.generate_tokens(&token_ids, params, &[], should_continue);
        self.model.clear_kv_cache();
        match &output {
            Ok(_) => self.phase = InferencePhase::Ready,
            Err(_) => self.phase = InferencePhase::Failed,
        }
        output
    }

    fn generate_tokens(
        &mut self,
        prompt_token_ids: &[u32],
        params: SamplingParams,
        stop_token_ids: &[u32],
        mut should_continue: impl FnMut() -> bool,
    ) -> Result<GenerationOutput, EngineError> {
        let mut sampler = Sampler::new(
            params,
            self.model.vocab_size(),
            self.tokenizer.eos_token_id,
            stop_token_ids.to_vec(),
            self.model.context_limit(),
            prompt_token_ids,
        )
        .map_err(EngineError::Message)?;
        let mut logits = self
            .model
            .prefill_logits(prompt_token_ids)
            .map_err(EngineError::from)?;
        let mut events: Vec<TokenResultEvent> = Vec::new();
        let mut generated_ids = Vec::new();
        let stop_reason;
        let request_id = RequestId::new();

        loop {
            if !should_continue() {
                stop_reason = StopReason::Cancelled;
                if let Some(last) = events.last_mut() {
                    last.is_last = true;
                    last.stop_reason = Some(StopReason::Cancelled);
                }
                break;
            }

            let outcome = sampler.sample(&logits).map_err(EngineError::Message)?;
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
            events.push(event);
            if outcome.is_last {
                stop_reason = outcome.stop_reason.unwrap_or(StopReason::Error);
                break;
            }
            logits = self
                .model
                .decode_logits(outcome.token_id)
                .map_err(EngineError::from)?;
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
}

fn locate_sidecar(
    cache_root: &Path,
    hf_repo_dir: Option<&Path>,
    repository: &str,
    revision: &str,
    file_name: &str,
) -> Result<PathBuf, EngineError> {
    if let Some(dir) = hf_repo_dir {
        let candidate = dir.join(file_name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    let direct = cache_root.join(file_name);
    if direct.is_file() {
        return Ok(direct);
    }

    // HF hub cache layout used by hf-hub 0.4
    let sanitized = repository.replace('/', "--");
    let hub_root = cache_root
        .parent()
        .unwrap_or(cache_root)
        .join("cache")
        .join("hf-hub")
        .join(format!("models--{sanitized}"))
        .join("snapshots")
        .join(revision)
        .join(file_name);
    if hub_root.is_file() {
        return Ok(hub_root);
    }

    // Also search common HF_HOME
    if let Ok(home) = std::env::var("HF_HOME") {
        let candidate = PathBuf::from(home)
            .join("hub")
            .join(format!("models--{sanitized}"))
            .join("snapshots")
            .join(revision)
            .join(file_name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        let candidate = PathBuf::from(home)
            .join(".cache/huggingface/hub")
            .join(format!("models--{sanitized}"))
            .join("snapshots")
            .join(revision)
            .join(file_name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }

    Err(EngineError::Message(format!(
        "missing {file_name} for {repository}@{revision}; resolve/prepare the model first"
    )))
}
