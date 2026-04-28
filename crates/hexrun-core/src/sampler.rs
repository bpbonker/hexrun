//! Token sampling: temperature, top-k, top-p (nucleus). The full
//! generation loop in `engine.rs` calls into this module after a forward
//! pass returns logits for the next position.
//!
//! Implementation is deterministic given a `rand::SeedableRng` seed so that
//! tests are reproducible.

use std::cmp::Ordering;

use thiserror::Error;

#[derive(Debug, Clone, Copy)]
pub struct SamplerConfig {
    /// Higher = more random. Must be >= 0. Zero means greedy (argmax).
    pub temperature: f32,
    /// Nucleus sampling threshold in (0, 1]. 1.0 disables.
    pub top_p: f32,
    /// Top-k pruning. 0 disables.
    pub top_k: usize,
}

impl Default for SamplerConfig {
    fn default() -> Self {
        Self {
            temperature: 0.7,
            top_p: 0.95,
            top_k: 40,
        }
    }
}

#[derive(Debug, Error)]
pub enum SamplerError {
    #[error("temperature {0} is negative")]
    NegativeTemperature(f32),
    #[error("top_p {0} not in (0, 1]")]
    BadTopP(f32),
    #[error("logits slice is empty")]
    EmptyLogits,
    #[error("logits contained non-finite values")]
    NonFiniteLogits,
}

impl SamplerConfig {
    pub fn validate(&self) -> Result<(), SamplerError> {
        if self.temperature < 0.0 {
            return Err(SamplerError::NegativeTemperature(self.temperature));
        }
        if self.top_p <= 0.0 || self.top_p > 1.0 {
            return Err(SamplerError::BadTopP(self.top_p));
        }
        Ok(())
    }
}

/// Sample a token id from `logits` according to `cfg`. `rng_uniform` is a
/// caller-supplied uniform draw in [0, 1) — pass a closure backed by your
/// preferred RNG.
pub fn sample(
    logits: &[f32],
    cfg: SamplerConfig,
    mut rng_uniform: impl FnMut() -> f32,
) -> Result<usize, SamplerError> {
    cfg.validate()?;
    if logits.is_empty() {
        return Err(SamplerError::EmptyLogits);
    }
    if logits.iter().any(|x| !x.is_finite()) {
        return Err(SamplerError::NonFiniteLogits);
    }

    if cfg.temperature == 0.0 {
        return Ok(argmax(logits));
    }

    let mut indexed: Vec<(usize, f32)> = logits
        .iter()
        .enumerate()
        .map(|(i, &x)| (i, x / cfg.temperature))
        .collect();

    indexed.sort_unstable_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(Ordering::Equal));

    if cfg.top_k > 0 && indexed.len() > cfg.top_k {
        indexed.truncate(cfg.top_k);
    }

    let max = indexed[0].1;
    let mut sum = 0.0f64;
    let mut probs: Vec<f64> = indexed
        .iter()
        .map(|&(_, x)| {
            let p = ((x - max) as f64).exp();
            sum += p;
            p
        })
        .collect();
    for p in probs.iter_mut() {
        *p /= sum;
    }

    if cfg.top_p < 1.0 {
        let mut cum = 0.0f64;
        let mut keep = probs.len();
        for (i, &p) in probs.iter().enumerate() {
            cum += p;
            if cum >= cfg.top_p as f64 {
                keep = i + 1;
                break;
            }
        }
        probs.truncate(keep);
        indexed.truncate(keep);
        let new_sum: f64 = probs.iter().sum();
        if new_sum > 0.0 {
            for p in probs.iter_mut() {
                *p /= new_sum;
            }
        }
    }

    let mut r = rng_uniform().clamp(0.0, 1.0 - f32::EPSILON) as f64;
    for (i, &p) in probs.iter().enumerate() {
        if r < p {
            return Ok(indexed[i].0);
        }
        r -= p;
    }
    Ok(indexed.last().expect("probs non-empty").0)
}

fn argmax(logits: &[f32]) -> usize {
    let mut best = 0usize;
    let mut best_v = f32::NEG_INFINITY;
    for (i, &v) in logits.iter().enumerate() {
        if v > best_v {
            best = i;
            best_v = v;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greedy_picks_argmax() {
        let logits = [0.1, 9.0, 0.5];
        let cfg = SamplerConfig {
            temperature: 0.0,
            top_p: 1.0,
            top_k: 0,
        };
        let out = sample(&logits, cfg, || 0.0).unwrap();
        assert_eq!(out, 1);
    }

    #[test]
    fn rejects_empty_logits() {
        let cfg = SamplerConfig::default();
        assert!(matches!(
            sample(&[], cfg, || 0.0),
            Err(SamplerError::EmptyLogits)
        ));
    }

    #[test]
    fn rejects_nan() {
        let cfg = SamplerConfig::default();
        let logits = [1.0, f32::NAN, 0.5];
        assert!(matches!(
            sample(&logits, cfg, || 0.0),
            Err(SamplerError::NonFiniteLogits)
        ));
    }

    #[test]
    fn rejects_negative_temperature() {
        let cfg = SamplerConfig {
            temperature: -1.0,
            ..Default::default()
        };
        assert!(matches!(cfg.validate(), Err(SamplerError::NegativeTemperature(_))));
    }

    #[test]
    fn rejects_bad_top_p() {
        let mut cfg = SamplerConfig::default();
        cfg.top_p = 1.5;
        assert!(matches!(cfg.validate(), Err(SamplerError::BadTopP(_))));
        cfg.top_p = 0.0;
        assert!(matches!(cfg.validate(), Err(SamplerError::BadTopP(_))));
    }

    #[test]
    fn top_k_one_is_deterministic() {
        let logits = [0.1, 9.0, 0.5, 7.0, 0.0];
        let cfg = SamplerConfig {
            temperature: 1.0,
            top_p: 1.0,
            top_k: 1,
        };
        for r in [0.0, 0.3, 0.7, 0.99] {
            let out = sample(&logits, cfg, || r).unwrap();
            assert_eq!(out, 1);
        }
    }

    #[test]
    fn top_p_respects_threshold() {
        // With strongly peaked logits, top_p=0.5 should keep only the top
        // bucket: argmax wins regardless of the random draw.
        let logits = [10.0, -10.0, -10.0, -10.0];
        let cfg = SamplerConfig {
            temperature: 1.0,
            top_p: 0.5,
            top_k: 0,
        };
        for r in [0.0, 0.4, 0.99] {
            let out = sample(&logits, cfg, || r).unwrap();
            assert_eq!(out, 0);
        }
    }

    #[test]
    fn distribution_is_well_formed_under_pressure() {
        // Sample many times and ensure we only ever see indices we expect.
        let logits = [3.0, 2.5, 1.0, 0.0];
        let cfg = SamplerConfig {
            temperature: 0.5,
            top_p: 1.0,
            top_k: 2,
        };
        let mut state: u32 = 0x1234_5678;
        let mut next = || {
            // xorshift32 — adequate for tests
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            (state as f32) / (u32::MAX as f32)
        };
        for _ in 0..1000 {
            let out = sample(&logits, cfg, &mut next).unwrap();
            assert!(out == 0 || out == 1, "got {out}");
        }
    }
}
