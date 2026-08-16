//! Exponential backoff with a visible countdown (spec §6.3 预算与限速).
//!
//! Free channels answer 429 with "免费额度限速,{n}s 后自动继续" instead of an
//! error; the state machine here is pure so the countdown UI and the retry
//! loop share one source of truth.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackoffPolicy {
    pub initial_secs: u64,
    pub max_secs: u64,
    /// After this many consecutive rate hits, suggest switching channels
    /// (顶部细条建议切换通道, §6.3).
    pub suggest_switch_after: u32,
}

impl Default for BackoffPolicy {
    fn default() -> Self {
        Self {
            initial_secs: 8,
            max_secs: 60,
            suggest_switch_after: 3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BackoffState {
    pub consecutive_hits: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackoffDecision {
    pub wait_secs: u64,
    pub suggest_channel_switch: bool,
}

impl BackoffState {
    /// Register a rate-limit hit. `server_retry_after` (from a Retry-After
    /// header) wins over the exponential schedule when present and sane.
    pub fn on_rate_limited(
        &mut self,
        policy: &BackoffPolicy,
        server_retry_after: Option<u64>,
    ) -> BackoffDecision {
        self.consecutive_hits += 1;
        let exp = policy
            .initial_secs
            .saturating_mul(1u64 << (self.consecutive_hits - 1).min(8))
            .min(policy.max_secs);
        let wait_secs = match server_retry_after {
            Some(s) if s > 0 => s.min(policy.max_secs.max(s.min(300))),
            _ => exp,
        };
        BackoffDecision {
            wait_secs,
            suggest_channel_switch: self.consecutive_hits >= policy.suggest_switch_after,
        }
    }

    /// A successful request resets the streak.
    pub fn on_success(&mut self) {
        self.consecutive_hits = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doubles_up_to_cap() {
        let p = BackoffPolicy::default();
        let mut s = BackoffState::default();
        assert_eq!(s.on_rate_limited(&p, None).wait_secs, 8);
        assert_eq!(s.on_rate_limited(&p, None).wait_secs, 16);
        assert_eq!(s.on_rate_limited(&p, None).wait_secs, 32);
        assert_eq!(s.on_rate_limited(&p, None).wait_secs, 60); // capped
        assert_eq!(s.on_rate_limited(&p, None).wait_secs, 60);
    }

    #[test]
    fn suggests_switch_after_three_hits() {
        let p = BackoffPolicy::default();
        let mut s = BackoffState::default();
        assert!(!s.on_rate_limited(&p, None).suggest_channel_switch);
        assert!(!s.on_rate_limited(&p, None).suggest_channel_switch);
        assert!(s.on_rate_limited(&p, None).suggest_channel_switch);
    }

    #[test]
    fn server_retry_after_wins() {
        let p = BackoffPolicy::default();
        let mut s = BackoffState::default();
        assert_eq!(s.on_rate_limited(&p, Some(20)).wait_secs, 20);
    }

    #[test]
    fn success_resets() {
        let p = BackoffPolicy::default();
        let mut s = BackoffState::default();
        s.on_rate_limited(&p, None);
        s.on_rate_limited(&p, None);
        s.on_success();
        assert_eq!(s.on_rate_limited(&p, None).wait_secs, 8);
    }
}
