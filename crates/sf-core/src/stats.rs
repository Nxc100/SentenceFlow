//! Report statistics folding (spec §4.5 报告).
//!
//! `fold_stats` turns the raw practice log into everything the report page
//! renders: today card, heat-map calendar, WPM/accuracy curves, weak-point
//! analysis by POS and sentence role, and the streak (火苗).
//!
//! Day boundaries are computed with an explicit timezone offset — the core
//! stays clock-free and identical on both native and wasm.

use crate::sentence::{PosTag, RoleTag};
use crate::srs::Mode;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogResult {
    Correct,
    Wrong,
}

/// A grammar tag attributed to one error (the word the user got wrong).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "t", content = "v", rename_all = "snake_case")]
pub enum ErrorTag {
    Pos(PosTag),
    Role(RoleTag),
}

/// Row shape of `log` in progress.db (§7.7), plus error attribution tags.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogRow {
    /// Unix seconds.
    pub ts: i64,
    pub sentence_id: i64,
    pub mode: Mode,
    pub result: LogResult,
    pub dur_ms: u32,
    /// Keystroke / word errors within this attempt.
    pub errors: u32,
    /// Words per minute for this attempt (0 for non-typing modes).
    pub wpm: f32,
    /// The user revealed the answer during this attempt (excluded from streak
    /// counting, §4.1).
    #[serde(default)]
    pub seen_answer: bool,
    /// POS/role tags of the words the errors occurred on.
    #[serde(default)]
    pub error_tags: Vec<ErrorTag>,
}

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct DayStats {
    pub attempts: u32,
    pub correct: u32,
    pub practice_ms: u64,
    /// Mean WPM over typing-family attempts that reported a wpm.
    pub avg_wpm: f32,
    /// correct / attempts, 0..=1.
    pub accuracy: f32,
}

/// Error-rate entry for the weak-point analysis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WeakPoint<T> {
    pub tag: T,
    pub errors: u32,
    /// Errors attributed to this tag / all attributed errors, 0..=1.
    pub share: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatsSummary {
    /// Local-day index (unix days since epoch, tz-adjusted) → that day's stats.
    pub days: BTreeMap<i64, DayStats>,
    /// Consecutive practised days ending at the newest log day (火苗).
    pub streak_days: u32,
    /// Longest correct streak across attempts (看过答案的答对不计,§4.1).
    pub best_correct_streak: u32,
    pub weak_pos: Vec<WeakPoint<PosTag>>,
    pub weak_roles: Vec<WeakPoint<RoleTag>>,
    pub total_attempts: u64,
    pub total_correct: u64,
}

/// Local-day index for a timestamp under a fixed tz offset (seconds).
pub fn day_index(ts: i64, tz_offset_secs: i32) -> i64 {
    (ts + i64::from(tz_offset_secs)).div_euclid(crate::srs::SECONDS_PER_DAY)
}

pub fn fold_stats(logs: impl IntoIterator<Item = LogRow>, tz_offset_secs: i32) -> StatsSummary {
    let mut days: BTreeMap<i64, (DayStats, u32 /*wpm_n*/, f64 /*wpm_sum*/)> = BTreeMap::new();
    let mut pos_errors: BTreeMap<PosTag, u32> = BTreeMap::new();
    let mut role_errors: BTreeMap<RoleTag, u32> = BTreeMap::new();
    let mut total_attempts = 0u64;
    let mut total_correct = 0u64;
    let mut cur_streak = 0u32;
    let mut best_streak = 0u32;

    for row in logs {
        total_attempts += 1;
        let day = day_index(row.ts, tz_offset_secs);
        let entry = days.entry(day).or_default();
        entry.0.attempts += 1;
        entry.0.practice_ms += u64::from(row.dur_ms);
        match row.result {
            LogResult::Correct => {
                total_correct += 1;
                entry.0.correct += 1;
                if row.seen_answer {
                    cur_streak = 0; // 看过答案:不计连对
                } else {
                    cur_streak += 1;
                    best_streak = best_streak.max(cur_streak);
                }
            }
            LogResult::Wrong => cur_streak = 0,
        }
        if row.wpm > 0.0 {
            entry.1 += 1;
            entry.2 += f64::from(row.wpm);
        }
        for tag in &row.error_tags {
            match tag {
                ErrorTag::Pos(p) => *pos_errors.entry(*p).or_default() += 1,
                ErrorTag::Role(r) => *role_errors.entry(*r).or_default() += 1,
            }
        }
    }

    // Finalize per-day derived numbers.
    let days: BTreeMap<i64, DayStats> = days
        .into_iter()
        .map(|(day, (mut d, n, sum))| {
            d.avg_wpm = if n > 0 {
                (sum / f64::from(n)) as f32
            } else {
                0.0
            };
            d.accuracy = if d.attempts > 0 {
                d.correct as f32 / d.attempts as f32
            } else {
                0.0
            };
            (day, d)
        })
        .collect();

    // Streak of consecutive practised days ending at the last practised day.
    let mut streak_days = 0u32;
    let mut expected: Option<i64> = None;
    for &day in days.keys().rev() {
        match expected {
            None => {
                streak_days = 1;
                expected = Some(day - 1);
            }
            Some(e) if day == e => {
                streak_days += 1;
                expected = Some(day - 1);
            }
            _ => break,
        }
    }

    fn rank<T>(m: BTreeMap<T, u32>) -> Vec<WeakPoint<T>> {
        let total: u32 = m.values().sum();
        let mut v: Vec<WeakPoint<T>> = m
            .into_iter()
            .map(|(tag, errors)| WeakPoint {
                tag,
                errors,
                share: if total > 0 {
                    errors as f32 / total as f32
                } else {
                    0.0
                },
            })
            .collect();
        v.sort_by_key(|w| std::cmp::Reverse(w.errors));
        v
    }

    StatsSummary {
        days,
        streak_days,
        best_correct_streak: best_streak,
        weak_pos: rank(pos_errors),
        weak_roles: rank(role_errors),
        total_attempts,
        total_correct,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::srs::SECONDS_PER_DAY;

    const D0: i64 = 1_700_000_000; // arbitrary anchor

    fn row(ts: i64, result: LogResult) -> LogRow {
        LogRow {
            ts,
            sentence_id: 1,
            mode: Mode::Typing,
            result,
            dur_ms: 5000,
            errors: if result == LogResult::Wrong { 1 } else { 0 },
            wpm: 40.0,
            seen_answer: false,
            error_tags: vec![],
        }
    }

    #[test]
    fn day_grouping_respects_tz() {
        // 23:30 UTC is the next local day at UTC+8.
        let ts = D0 - (D0 % SECONDS_PER_DAY) + 23 * 3600 + 1800;
        assert_eq!(day_index(ts, 0) + 1, day_index(ts, 8 * 3600));
    }

    #[test]
    fn folds_days_and_accuracy() {
        let logs = vec![
            row(D0, LogResult::Correct),
            row(D0 + 60, LogResult::Wrong),
            row(D0 + SECONDS_PER_DAY, LogResult::Correct),
        ];
        let s = fold_stats(logs, 0);
        assert_eq!(s.days.len(), 2);
        assert_eq!(s.total_attempts, 3);
        assert_eq!(s.total_correct, 2);
        let first = s.days.values().next().unwrap();
        assert_eq!(first.attempts, 2);
        assert!((first.accuracy - 0.5).abs() < 1e-6);
        assert!((first.avg_wpm - 40.0).abs() < 1e-6);
    }

    #[test]
    fn day_streak_counts_consecutive_tail() {
        let logs = vec![
            row(D0, LogResult::Correct),                       // day 0
            row(D0 + 3 * SECONDS_PER_DAY, LogResult::Correct), // day 3
            row(D0 + 4 * SECONDS_PER_DAY, LogResult::Correct), // day 4
        ];
        let s = fold_stats(logs, 0);
        assert_eq!(s.streak_days, 2); // days 3–4; the gap breaks day 0
    }

    #[test]
    fn seen_answer_breaks_correct_streak() {
        let mut r1 = row(D0, LogResult::Correct);
        let mut r2 = row(D0 + 1, LogResult::Correct);
        r2.seen_answer = true;
        let r3 = row(D0 + 2, LogResult::Correct);
        r1.seen_answer = false;
        let s = fold_stats(vec![r1, r2, r3], 0);
        assert_eq!(s.best_correct_streak, 1);
    }

    #[test]
    fn weak_points_ranked_by_errors() {
        let mut r1 = row(D0, LogResult::Wrong);
        r1.error_tags = vec![
            ErrorTag::Pos(PosTag::Preposition),
            ErrorTag::Pos(PosTag::Article),
        ];
        let mut r2 = row(D0 + 1, LogResult::Wrong);
        r2.error_tags = vec![
            ErrorTag::Pos(PosTag::Preposition),
            ErrorTag::Role(RoleTag::Adverbial),
        ];
        let s = fold_stats(vec![r1, r2], 0);
        assert_eq!(s.weak_pos[0].tag, PosTag::Preposition);
        assert_eq!(s.weak_pos[0].errors, 2);
        assert!((s.weak_pos[0].share - 2.0 / 3.0).abs() < 1e-6);
        assert_eq!(s.weak_roles[0].tag, RoleTag::Adverbial);
    }
}
