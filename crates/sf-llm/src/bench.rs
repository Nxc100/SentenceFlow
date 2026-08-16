//! Free-model micro-benchmark scoring (spec §3.5 能力微基准).
//!
//! Once a channel is ready, each candidate model generates 6 sentences; the
//! local validator grades the output. This module owns the *scoring* —
//! running the bench is orchestration (desktop app), grading inputs come from
//! sf-pipeline verdicts.

use serde::{Deserialize, Serialize};

/// Raw measurements for one candidate model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchSample {
    pub model: String,
    /// Sentences requested (typically 6).
    pub requested: u32,
    /// Drafts that parsed as JSON at all.
    pub parsed: u32,
    /// Drafts that fully passed validation.
    pub passed: u32,
    /// Drafts flagged over-level.
    pub over_level: u32,
    /// Wall-clock latency of the request, ms.
    pub latency_ms: u64,
}

/// Scored result, persisted to progress.db `bench` (§7.7).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BenchScore {
    pub model: String,
    /// 0–100, higher is better.
    pub score: f32,
    pub json_ok_rate: f32,
    pub pass_rate: f32,
    pub over_level_rate: f32,
    pub latency_ms: u64,
}

/// Score one sample. Weights: JSON discipline 40%, validation pass 40%,
/// over-level penalty 10%, latency 10% (≤5s full marks, ≥60s zero).
pub fn score(sample: &BenchSample) -> BenchScore {
    let req = sample.requested.max(1) as f32;
    let json_ok_rate = (sample.parsed as f32 / req).clamp(0.0, 1.0);
    let pass_rate = (sample.passed as f32 / req).clamp(0.0, 1.0);
    let over_level_rate = (sample.over_level as f32 / req).clamp(0.0, 1.0);
    let latency_score = if sample.latency_ms <= 5_000 {
        1.0
    } else if sample.latency_ms >= 60_000 {
        0.0
    } else {
        1.0 - (sample.latency_ms - 5_000) as f32 / 55_000.0
    };
    let score = 100.0
        * (0.4 * json_ok_rate
            + 0.4 * pass_rate
            + 0.1 * (1.0 - over_level_rate)
            + 0.1 * latency_score);
    BenchScore {
        model: sample.model.clone(),
        score,
        json_ok_rate,
        pass_rate,
        over_level_rate,
        latency_ms: sample.latency_ms,
    }
}

/// Rank candidates: best first; 生成工坊 defaults to the top entry (§3.5).
pub fn rank(samples: &[BenchSample]) -> Vec<BenchScore> {
    let mut scored: Vec<BenchScore> = samples.iter().map(score).collect();
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored
}

/// Does a stored bench result still apply? (名单变化自动重测, §3.5)
pub fn needs_retest(stored_fingerprint: &str, current_fingerprint: &str) -> bool {
    stored_fingerprint != current_fingerprint
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(model: &str, parsed: u32, passed: u32, over: u32, latency: u64) -> BenchSample {
        BenchSample {
            model: model.into(),
            requested: 6,
            parsed,
            passed,
            over_level: over,
            latency_ms: latency,
        }
    }

    #[test]
    fn perfect_model_scores_high() {
        let s = score(&sample("good", 6, 6, 0, 3_000));
        assert!(s.score > 95.0, "score = {}", s.score);
    }

    #[test]
    fn json_discipline_dominates() {
        let sloppy = score(&sample("sloppy", 2, 1, 0, 3_000));
        let tidy = score(&sample("tidy", 6, 5, 1, 3_000));
        assert!(tidy.score > sloppy.score + 20.0);
    }

    #[test]
    fn ranking_orders_best_first() {
        let ranked = rank(&[
            sample("slow", 6, 6, 0, 59_000),
            sample("fast", 6, 6, 0, 2_000),
            sample("broken", 0, 0, 0, 2_000),
        ]);
        assert_eq!(ranked[0].model, "fast");
        assert_eq!(ranked[2].model, "broken");
    }

    #[test]
    fn fingerprint_change_triggers_retest() {
        assert!(needs_retest("abc", "def"));
        assert!(!needs_retest("abc", "abc"));
    }
}
