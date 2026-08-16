//! Dual-track metering (spec §7.5): 永不静默烧钱 (§1.4 第五条).
//!
//! * [`MoneyMeter`] — paid channels: pre-flight estimate, live accumulation
//!   during streaming, **hard stop** at the per-run budget with a graceful
//!   wind-down (流中触顶优雅收尾).
//! * [`RateBudget`] — free channels: client-side token bucket for requests
//!   per minute plus a daily request counter for the CostBar's 免费形态.
//!
//! Both are pure state machines; wall-clock time is always passed in.

use crate::estimate::cost_cny;
use serde::{Deserialize, Serialize};

/// Per-1M-token prices in CNY (user-editable, channels.json-refreshable §4.7).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PriceTable {
    pub prompt_per_m: f64,
    pub completion_per_m: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetVerdict {
    /// Under 80% of budget.
    Ok,
    /// ≥80% — CostBar turns warning (§5.5).
    Warning,
    /// Budget reached — stop the stream gracefully, keep what was produced.
    Exhausted,
}

/// Money metering for one generation run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MoneyMeter {
    pub prices: PriceTable,
    /// 单次上限,默认 ¥1 (§4.7).
    pub budget_cny: f64,
    /// Estimated prompt tokens (pre-flight).
    pub est_prompt_tokens: u64,
    /// Live completion tokens (estimated from streamed text).
    pub live_completion_tokens: u64,
    /// Authoritative usage once the backend reports it (回填).
    pub reported: Option<(u64, u64)>,
}

impl MoneyMeter {
    pub fn new(prices: PriceTable, budget_cny: f64, est_prompt_tokens: u64) -> Self {
        Self {
            prices,
            budget_cny,
            est_prompt_tokens,
            live_completion_tokens: 0,
            reported: None,
        }
    }

    /// Pre-flight estimate shown on the CostBar before [生成] is pressed:
    /// prompt tokens + an assumed completion of `expected_completion` tokens.
    pub fn preflight_cost(&self, expected_completion: u64) -> f64 {
        cost_cny(self.est_prompt_tokens, self.prices.prompt_per_m)
            + cost_cny(expected_completion, self.prices.completion_per_m)
    }

    /// Record streamed-out tokens; returns the current verdict so the caller
    /// can stop the stream the moment the budget is gone.
    pub fn add_completion_tokens(&mut self, tokens: u64) -> BudgetVerdict {
        self.live_completion_tokens += tokens;
        self.verdict()
    }

    /// Backfill authoritative usage from the API response.
    pub fn report_usage(&mut self, prompt_tokens: u64, completion_tokens: u64) {
        self.reported = Some((prompt_tokens, completion_tokens));
    }

    /// Best current cost: reported usage wins over estimates.
    pub fn current_cost(&self) -> f64 {
        match self.reported {
            Some((p, c)) => {
                cost_cny(p, self.prices.prompt_per_m) + cost_cny(c, self.prices.completion_per_m)
            }
            None => {
                cost_cny(self.est_prompt_tokens, self.prices.prompt_per_m)
                    + cost_cny(self.live_completion_tokens, self.prices.completion_per_m)
            }
        }
    }

    pub fn verdict(&self) -> BudgetVerdict {
        let cost = self.current_cost();
        if cost >= self.budget_cny {
            BudgetVerdict::Exhausted
        } else if cost >= self.budget_cny * 0.8 {
            BudgetVerdict::Warning
        } else {
            BudgetVerdict::Ok
        }
    }
}

/// Request-rate budget for free channels: a token bucket over a rolling
/// window plus a daily counter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RateBudget {
    /// Bucket capacity (requests per minute estimate).
    pub rpm: u32,
    /// Current tokens (fractional refill).
    tokens: f64,
    /// Unix seconds of last refill.
    last_refill: i64,
    /// Requests made today (CostBar 免费形态右侧, §5.5).
    pub today_requests: u32,
    /// Local day index of `today_requests`.
    today_day: i64,
}

impl RateBudget {
    pub fn new(rpm: u32, now: i64) -> Self {
        Self {
            rpm,
            tokens: f64::from(rpm),
            last_refill: now,
            today_requests: 0,
            today_day: now.div_euclid(86_400),
        }
    }

    fn refill(&mut self, now: i64) {
        let elapsed = (now - self.last_refill).max(0) as f64;
        self.tokens = (self.tokens + elapsed * f64::from(self.rpm) / 60.0).min(f64::from(self.rpm));
        self.last_refill = now;
        let day = now.div_euclid(86_400);
        if day != self.today_day {
            self.today_day = day;
            self.today_requests = 0;
        }
    }

    /// Try to take one request slot. `Ok` on success; `Err(wait_secs)` with
    /// the seconds until a slot frees up otherwise.
    pub fn try_acquire(&mut self, now: i64) -> Result<(), u64> {
        self.refill(now);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            self.today_requests += 1;
            Ok(())
        } else {
            let missing = 1.0 - self.tokens;
            Err((missing * 60.0 / f64::from(self.rpm)).ceil() as u64)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prices() -> PriceTable {
        PriceTable {
            prompt_per_m: 2.0,
            completion_per_m: 8.0,
        }
    }

    #[test]
    fn preflight_math() {
        let m = MoneyMeter::new(prices(), 1.0, 45_000);
        // 45K prompt at ¥2/M + 10K completion at ¥8/M = 0.09 + 0.08
        assert!((m.preflight_cost(10_000) - 0.17).abs() < 1e-9);
    }

    #[test]
    fn warning_at_80_percent_exhausted_at_budget() {
        let mut m = MoneyMeter::new(prices(), 1.0, 0);
        // completion ¥8/M → 100K tokens = ¥0.8 (warning), 125K = ¥1.0 (stop)
        assert_eq!(m.add_completion_tokens(99_000), BudgetVerdict::Ok);
        assert_eq!(m.add_completion_tokens(1_000), BudgetVerdict::Warning);
        assert_eq!(m.add_completion_tokens(25_000), BudgetVerdict::Exhausted);
    }

    #[test]
    fn reported_usage_wins_over_estimate() {
        let mut m = MoneyMeter::new(prices(), 1.0, 1_000_000); // wild estimate
        m.report_usage(10_000, 5_000);
        // 10K*2/M + 5K*8/M = 0.02 + 0.04
        assert!((m.current_cost() - 0.06).abs() < 1e-9);
        assert_eq!(m.verdict(), BudgetVerdict::Ok);
    }

    #[test]
    fn rate_bucket_blocks_then_refills() {
        let mut r = RateBudget::new(6, 0); // 6 rpm = one per 10s
        for _ in 0..6 {
            assert!(r.try_acquire(0).is_ok());
        }
        let wait = r.try_acquire(0).unwrap_err();
        assert_eq!(wait, 10);
        assert!(r.try_acquire(10).is_ok()); // refilled one slot
        assert_eq!(r.today_requests, 7);
    }

    #[test]
    fn daily_counter_resets_across_days() {
        let mut r = RateBudget::new(60, 0);
        assert!(r.try_acquire(0).is_ok());
        assert_eq!(r.today_requests, 1);
        assert!(r.try_acquire(86_400 + 1).is_ok());
        assert_eq!(r.today_requests, 1); // new day restarted the count
    }
}
