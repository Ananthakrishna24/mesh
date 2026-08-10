mod engine;
mod pipeline;
mod manager;
mod sampler;
mod tokenizer;

pub use engine::{load_mesh_tokenizer, EngineError, GenerationOutput, SingleNodeEngine};
pub use manager::{LocalResourceManager, ReserveOutcome};
pub use pipeline::{PipelineEngine, PipelineError, StageActivation, StageHop, StageWorker};
pub use sampler::{SampleOutcome, Sampler};
pub use tokenizer::{render_non_thinking_chat, MeshTokenizer, TokenizerError};

pub fn crate_name() -> &'static str {
    "mesh-inference"
}
