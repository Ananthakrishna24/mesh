use std::cmp::Ordering;

use mesh_core::{SamplingParams, StopReason};
use rand::Rng;
use rand::SeedableRng;
use rand_chacha::ChaCha12Rng;

#[derive(Debug, Clone)]
pub struct Sampler {
    params: SamplingParams,
    rng: ChaCha12Rng,
    history: Vec<u32>,
    generated: u32,
    vocab_size: u32,
    eos_token_id: u32,
    stop_token_ids: Vec<u32>,
    context_limit: u32,
    prompt_len: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SampleOutcome {
    pub token_id: u32,
    pub token_index: u32,
    pub is_last: bool,
    pub stop_reason: Option<StopReason>,
    pub sequence_length: u32,
}

impl Sampler {
    pub fn new(
        params: SamplingParams,
        vocab_size: u32,
        eos_token_id: u32,
        stop_token_ids: Vec<u32>,
        context_limit: u32,
        prompt_token_ids: &[u32],
    ) -> Result<Self, String> {
        params.validate(prompt_token_ids.len() as u32, context_limit, vocab_size)?;
        Ok(Self {
            rng: ChaCha12Rng::seed_from_u64(params.seed),
            params,
            history: prompt_token_ids.to_vec(),
            generated: 0,
            vocab_size,
            eos_token_id,
            stop_token_ids,
            context_limit,
            prompt_len: prompt_token_ids.len() as u32,
        })
    }

    pub fn sample(&mut self, logits: &[f32]) -> Result<SampleOutcome, String> {
        if logits.len() as u32 != self.vocab_size {
            return Err(format!(
                "logits length {} does not match vocab_size {}",
                logits.len(),
                self.vocab_size
            ));
        }
        let mut scores = logits.to_vec();
        apply_repetition_penalty(&mut scores, &self.history, self.params.repetition_penalty);

        let token_id = if self.params.temperature == 0.0 {
            greedy_argmax(&scores)
        } else {
            apply_temperature(&mut scores, self.params.temperature);
            if self.params.top_k > 0 {
                apply_top_k(&mut scores, self.params.top_k as usize);
            }
            let probs = if self.params.top_p < 1.0 {
                let mut probs = softmax_finite(&scores)?;
                apply_top_p(&mut probs, self.params.top_p)?;
                probs
            } else {
                softmax_finite(&scores)?
            };
            categorical_sample(&probs, &mut self.rng)?
        };

        if token_id >= self.vocab_size {
            return Err(format!("sampled token {token_id} outside vocab"));
        }

        self.history.push(token_id);
        let token_index = self.generated;
        self.generated = self.generated.saturating_add(1);
        let sequence_length = self.prompt_len.saturating_add(self.generated);

        let mut stop_reason = None;
        if token_id == self.eos_token_id || self.stop_token_ids.contains(&token_id) {
            stop_reason = Some(StopReason::Eos);
        } else if self.generated >= self.params.max_new_tokens {
            stop_reason = Some(StopReason::MaxNewTokens);
        } else if sequence_length >= self.context_limit {
            stop_reason = Some(StopReason::ContextLimit);
        }

        Ok(SampleOutcome {
            token_id,
            token_index,
            is_last: stop_reason.is_some(),
            stop_reason,
            sequence_length,
        })
    }
}

fn apply_repetition_penalty(scores: &mut [f32], history: &[u32], penalty: f32) {
    if (penalty - 1.0).abs() < f32::EPSILON {
        return;
    }
    let mut seen = vec![false; scores.len()];
    for &token in history {
        let idx = token as usize;
        if idx >= scores.len() || seen[idx] {
            continue;
        }
        seen[idx] = true;
        if scores[idx] > 0.0 {
            scores[idx] /= penalty;
        } else {
            scores[idx] *= penalty;
        }
    }
}

fn apply_temperature(scores: &mut [f32], temperature: f32) {
    let temperature = temperature.max(1e-5);
    for score in scores.iter_mut() {
        *score /= temperature;
    }
}

fn greedy_argmax(scores: &[f32]) -> u32 {
    let mut best_idx = 0usize;
    let mut best_val = f32::NEG_INFINITY;
    for (idx, &value) in scores.iter().enumerate() {
        if !value.is_finite() {
            continue;
        }
        if value > best_val || (value == best_val && idx < best_idx) {
            best_val = value;
            best_idx = idx;
        }
    }
    best_idx as u32
}

fn apply_top_k(scores: &mut [f32], k: usize) {
    if k == 0 || k >= scores.len() {
        return;
    }
    let mut indices = (0..scores.len()).collect::<Vec<_>>();
    indices.sort_by(|&left, &right| {
        match scores[right]
            .partial_cmp(&scores[left])
            .unwrap_or(Ordering::Equal)
        {
            Ordering::Equal => left.cmp(&right),
            other => other,
        }
    });
    let mut keep = vec![false; scores.len()];
    for idx in indices.into_iter().take(k) {
        keep[idx] = true;
    }
    for (idx, score) in scores.iter_mut().enumerate() {
        if !keep[idx] {
            *score = f32::NEG_INFINITY;
        }
    }
}

fn softmax_finite(scores: &[f32]) -> Result<Vec<f32>, String> {
    let mut max = f32::NEG_INFINITY;
    for &score in scores {
        if score.is_finite() && score > max {
            max = score;
        }
    }
    if !max.is_finite() {
        return Err("no finite logits after filtering".to_owned());
    }
    let mut values = vec![0.0f32; scores.len()];
    let mut sum = 0.0f64;
    for (idx, &score) in scores.iter().enumerate() {
        if !score.is_finite() {
            continue;
        }
        let exp = ((score - max) as f64).exp();
        values[idx] = exp as f32;
        sum += exp;
    }
    if sum <= 0.0 {
        return Err("softmax mass was empty".to_owned());
    }
    for value in &mut values {
        *value = (*value as f64 / sum) as f32;
    }
    Ok(values)
}

fn apply_top_p(probs: &mut [f32], top_p: f32) -> Result<(), String> {
    let mut indexed = probs
        .iter()
        .enumerate()
        .filter(|(_, value)| **value > 0.0)
        .map(|(idx, value)| (idx, *value))
        .collect::<Vec<_>>();
    if indexed.is_empty() {
        return Err("top-p received empty distribution".to_owned());
    }
    indexed.sort_by(
        |left, right| match right.1.partial_cmp(&left.1).unwrap_or(Ordering::Equal) {
            Ordering::Equal => left.0.cmp(&right.0),
            other => other,
        },
    );
    let mut cumulative = 0.0f32;
    let mut keep_count = 0usize;
    for (_, prob) in &indexed {
        keep_count += 1;
        cumulative += *prob;
        if cumulative >= top_p {
            break;
        }
    }
    keep_count = keep_count.max(1);
    let mut keep = vec![false; probs.len()];
    for (idx, _) in indexed.into_iter().take(keep_count) {
        keep[idx] = true;
    }
    let mut sum = 0.0f32;
    for (idx, prob) in probs.iter_mut().enumerate() {
        if keep[idx] {
            sum += *prob;
        } else {
            *prob = 0.0;
        }
    }
    if sum <= 0.0 {
        return Err("top-p renormalization failed".to_owned());
    }
    for prob in probs.iter_mut() {
        *prob /= sum;
    }
    Ok(())
}

fn categorical_sample(probs: &[f32], rng: &mut ChaCha12Rng) -> Result<u32, String> {
    let draw: f64 = rng.random::<f64>();
    let mut cumulative = 0.0f64;
    let mut last_positive = None;
    for (idx, &prob) in probs.iter().enumerate() {
        if prob <= 0.0 {
            continue;
        }
        last_positive = Some(idx as u32);
        cumulative += f64::from(prob);
        if draw <= cumulative {
            return Ok(idx as u32);
        }
    }
    last_positive.ok_or_else(|| "categorical sample found no mass".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn temperature_zero_is_greedy_and_stable() {
        let params = SamplingParams {
            temperature: 0.0,
            top_k: 0,
            top_p: 1.0,
            repetition_penalty: 1.0,
            seed: 7,
            max_new_tokens: 4,
        };
        let prompt = vec![1u32, 2, 3];
        let mut sampler = Sampler::new(params, 8, 7, Vec::new(), 64, &prompt).expect("sampler");
        let logits = vec![0.1, 0.2, 5.0, 0.4, 0.0, -1.0, 1.0, 0.3];
        let first = sampler.sample(&logits).expect("sample");
        assert_eq!(first.token_id, 2);
        assert!(!first.is_last);

        let mut sampler2 = Sampler::new(params, 8, 7, Vec::new(), 64, &prompt).expect("sampler");
        let second = sampler2.sample(&logits).expect("sample");
        assert_eq!(first, second);
    }

    #[test]
    fn eos_marks_last() {
        let params = SamplingParams {
            temperature: 0.0,
            top_k: 0,
            top_p: 1.0,
            repetition_penalty: 1.0,
            seed: 1,
            max_new_tokens: 8,
        };
        let mut sampler = Sampler::new(params, 4, 2, Vec::new(), 64, &[0]).expect("sampler");
        let logits = vec![0.0, 0.0, 10.0, 0.0];
        let outcome = sampler.sample(&logits).expect("sample");
        assert_eq!(outcome.token_id, 2);
        assert!(outcome.is_last);
        assert_eq!(outcome.stop_reason, Some(StopReason::Eos));
    }

    #[test]
    fn repetition_penalty_can_change_greedy_choice() {
        let params = SamplingParams {
            temperature: 0.0,
            top_k: 0,
            top_p: 1.0,
            repetition_penalty: 2.0,
            seed: 1,
            max_new_tokens: 4,
        };
        let mut sampler = Sampler::new(params, 3, 99, Vec::new(), 64, &[1]).expect("sampler");
        let logits = vec![1.0, 1.5, 0.5];
        let outcome = sampler.sample(&logits).expect("sample");
        assert_eq!(outcome.token_id, 0);
    }
}
