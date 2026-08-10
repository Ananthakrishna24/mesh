mod engine;
mod manager;
mod pipeline;
mod sampler;
mod tokenizer;

pub use engine::{EngineError, GenerationOutput, SingleNodeEngine, load_mesh_tokenizer};
pub use manager::{LocalResourceManager, ReserveOutcome};
pub use pipeline::{PipelineEngine, PipelineError, StageActivation, StageHop, StageWorker};
pub use sampler::{SampleOutcome, Sampler};
pub use tokenizer::{MeshTokenizer, TokenizerError, render_non_thinking_chat};

pub fn crate_name() -> &'static str {
    "mesh-inference"
}
