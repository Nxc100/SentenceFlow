//! Trial state machine (spec §4.6 / §7.6).
//!
//! 14 天全功能试用;首启写起点(app 层双份持久化:文件 + 钥匙串);
//! `now < last_seen` 判时钟回拨即到期 — 不做更深对抗。
//! 到期后进入体验模式(每日 5 句),不清数据、不弹窗。

use serde::{Deserialize, Serialize};

pub const TRIAL_DAYS: i64 = 14;
/// 体验模式每日句数上限 (spec §4.6).
pub const LAPSED_DAILY_SENTENCES: u32 = 5;
const SECONDS_PER_DAY: i64 = 86_400;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrialState {
    /// Unix seconds of first launch.
    pub started_at: i64,
    /// Highest clock value ever observed (rollback detector).
    pub last_seen: i64,
    /// Set permanently once a rollback is detected.
    #[serde(default)]
    pub rolled_back: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TrialVerdict {
    /// 试用中,剩 `days_left` 天(向上取整,最后一天显示 1).
    Active { days_left: u32 },
    /// 正常到期 → 体验模式.
    Expired,
    /// 时钟回拨 → 即刻到期(§6.4:授权页说明原因,无指责性文案).
    ExpiredClockRollback,
}

impl TrialState {
    pub fn start(now: i64) -> Self {
        Self {
            started_at: now,
            last_seen: now,
            rolled_back: false,
        }
    }

    /// Advance the observed clock and evaluate the trial. Call on every app
    /// start and before any gated action; persist the returned state.
    pub fn advance(mut self, now: i64) -> (Self, TrialVerdict) {
        if now < self.last_seen {
            self.rolled_back = true;
        } else {
            self.last_seen = now;
        }
        let verdict = if self.rolled_back {
            TrialVerdict::ExpiredClockRollback
        } else {
            let elapsed = now - self.started_at;
            let remaining = TRIAL_DAYS * SECONDS_PER_DAY - elapsed;
            if remaining <= 0 {
                TrialVerdict::Expired
            } else {
                // Ceiling division; `remaining` is positive here.
                TrialVerdict::Active {
                    days_left: ((remaining + SECONDS_PER_DAY - 1) / SECONDS_PER_DAY) as u32,
                }
            }
        };
        (self, verdict)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_700_000_000;

    #[test]
    fn fresh_trial_has_14_days() {
        let (_, v) = TrialState::start(T0).advance(T0);
        assert_eq!(v, TrialVerdict::Active { days_left: 14 });
    }

    #[test]
    fn last_day_shows_one() {
        let s = TrialState::start(T0);
        let (_, v) = s.advance(T0 + (TRIAL_DAYS * SECONDS_PER_DAY) - 30);
        assert_eq!(v, TrialVerdict::Active { days_left: 1 });
    }

    #[test]
    fn expires_after_14_days() {
        let s = TrialState::start(T0);
        let (_, v) = s.advance(T0 + TRIAL_DAYS * SECONDS_PER_DAY);
        assert_eq!(v, TrialVerdict::Expired);
    }

    #[test]
    fn clock_rollback_expires_immediately_and_sticks() {
        let s = TrialState::start(T0);
        let (s, _) = s.advance(T0 + 3 * SECONDS_PER_DAY);
        let (s, v) = s.advance(T0 + SECONDS_PER_DAY); // rolled back 2 days
        assert_eq!(v, TrialVerdict::ExpiredClockRollback);
        // Even after the clock moves forward again, the verdict stays.
        let (_, v) = s.advance(T0 + 5 * SECONDS_PER_DAY);
        assert_eq!(v, TrialVerdict::ExpiredClockRollback);
    }

    #[test]
    fn monotonic_clock_never_triggers_rollback() {
        let mut s = TrialState::start(T0);
        for d in 0..10 {
            let (next, v) = s.advance(T0 + d * SECONDS_PER_DAY);
            assert_ne!(v, TrialVerdict::ExpiredClockRollback);
            s = next;
        }
    }
}
