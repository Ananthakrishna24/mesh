use std::path::Path;

use mesh_model::hash_bytes_hex;
use thiserror::Error;
use tokenizers::Tokenizer;

#[derive(Debug, Error)]
pub enum TokenizerError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(Clone)]
pub struct MeshTokenizer {
    tokenizer: Tokenizer,
    pub tokenizer_hash: String,
    pub eos_token_id: u32,
}

impl MeshTokenizer {
    pub fn load(tokenizer_json: &Path, expected_hash: &str, eos_token_id: u32) -> Result<Self, TokenizerError> {
        let bytes = std::fs::read(tokenizer_json)?;
        let actual = hash_bytes_hex(&bytes);
        if !expected_hash.is_empty() && actual != expected_hash {
            return Err(TokenizerError::Message(format!(
                "tokenizer_hash mismatch: expected {expected_hash}, got {actual}"
            )));
        }
        let tokenizer = Tokenizer::from_bytes(&bytes).map_err(|error| {
            TokenizerError::Message(format!("failed to parse tokenizer.json: {error}"))
        })?;
        Ok(Self {
            tokenizer,
            tokenizer_hash: actual,
            eos_token_id,
        })
    }

    pub fn encode_chat(
        &self,
        system: Option<&str>,
        user: &str,
    ) -> Result<Vec<u32>, TokenizerError> {
        let prompt = render_non_thinking_chat(system, user);
        self.encode_text(&prompt)
    }

    pub fn encode_text(&self, text: &str) -> Result<Vec<u32>, TokenizerError> {
        let encoding = self
            .tokenizer
            .encode(text, false)
            .map_err(|error| TokenizerError::Message(error.to_string()))?;
        Ok(encoding.get_ids().to_vec())
    }

    pub fn decode_stream(&self, token_ids: &[u32]) -> Result<String, TokenizerError> {
        self.tokenizer
            .decode(token_ids, true)
            .map_err(|error| TokenizerError::Message(error.to_string()))
    }
}

pub fn render_non_thinking_chat(system: Option<&str>, user: &str) -> String {
    let mut out = String::new();
    if let Some(system) = system.map(str::trim).filter(|value| !value.is_empty()) {
        out.push_str("<|im_start|>system\n");
        out.push_str(system);
        out.push_str("<|im_end|>\n");
    }
    out.push_str("<|im_start|>user\n");
    out.push_str(user.trim());
    out.push_str("<|im_end|>\n");
    // Official Qwen3 non-thinking generation prompt: empty think block forces direct answer.
    out.push_str("<|im_start|>assistant\n<think>\n\n</think>\n\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn non_thinking_template_ends_ready_for_assistant() {
        let rendered = render_non_thinking_chat(Some("sys"), "hello");
        assert!(rendered.starts_with("<|im_start|>system\n"));
        assert!(rendered.contains("<|im_start|>user\nhello<|im_end|>\n"));
        assert!(
            rendered.ends_with("<|im_start|>assistant\n<think>\n\n</think>\n\n"),
            "expected official Qwen3 non-thinking suffix, got {rendered:?}"
        );
    }
}
