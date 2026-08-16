//! Leitner five-box SRS (spec §4.5), interpreted through the LevelSpec.
//!
//! Design notes:
//! * "听打答对按 1.5× 权重推进" is modelled as fractional progress inside a
//!   box: a correct answer adds its mode weight; each whole 1.0 advances one
//!   box. A plain typing correct (weight 1.0) therefore advances exactly one
//!   box, while listening corrects reach the next box faster over time.
//! * "看过答案" (revealed answer) still counts for SRS advancement but is
//!   recorded on the state so stats can exclude it from streaks (spec §4.1).

use crate::spec::LevelSpec;
use serde::{Deserialize, Serialize};

pub const SECONDS_PER_DAY: i64 = 86_400;

/// Practice modes (spec §4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// 打字.
    Typing,
    /// 拆句重组.
    Reorder,
    /// 听打.
    Listening,
    /// 默写.
    Dictation,
}

/// Per-sentence SRS state — the row shape of `srs` in progress.db (§7.7).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SrsState {
    /// Current box, 1..=5.
    pub box_idx: u8,
    /// Fractional progress toward the next box (see module docs).
    pub progress: f32,
    /// Unix seconds when the sentence is next due.
    pub due_at: i64,
    /// Lifetime wrong-answer count (错题本 keys off this, §4.3).
    pub err: u32,
    pub last_mode: Option<Mode>,
    pub last_at: i64,
    /// The user revealed the answer at least once for this sentence.
    pub seen_answer: bool,
    /// User pressed Ctrl+Q (不熟悉) at least once.
    pub marked_unfamiliar: bool,
}

impl SrsState {
    /// A sentence never practised: box 1, due immediately.
    pub fn new(now: i64) -> Self {
        Self {
            box_idx: 1,
            progress: 0.0,
            due_at: now,
            err: 0,
            last_mode: None,
            last_at: now,
            seen_answer: false,
            marked_unfamiliar: false,
        }
    }

    /// 掌握口径:盒 ≥ 4 (spec §4.5).
    pub fn is_mastered(&self) -> bool {
        self.box_idx >= 4
    }

    /// 错题本收录条件:错 ≥2 次或标"不熟悉" (spec §4.3).
    pub fn qualifies_for_wrongbook(&self) -> bool {
        self.err >= 2 || self.marked_unfamiliar
    }

    pub fn is_due(&self, now: i64) -> bool {
        self.due_at <= now
    }
}

/// What happened when the user finished (or acted on) a sentence.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Outcome {
    /// Whole sentence correct. `seen_answer`: the reveal key (↓) was used.
    Correct { seen_answer: bool },
    /// Sentence failed (宽松模式 Enter 校验未过 / 默写 diff 有错).
    Wrong,
    /// Ctrl+M — 直入盒 5.
    MarkMastered,
    /// Ctrl+Q — 回盒 1,收录错题本.
    MarkUnfamiliar,
}

/// Advance one sentence's SRS state by one outcome. Pure; storage happens
/// outside.
pub fn apply_outcome(
    state: SrsState,
    outcome: Outcome,
    mode: Mode,
    spec: &LevelSpec,
    now: i64,
) -> SrsState {
    let mut s = state;
    s.last_mode = Some(mode);
    s.last_at = now;

    match outcome {
        Outcome::Correct { seen_answer } => {
            if seen_answer {
                s.seen_answer = true;
            }
            let weight = if mode == Mode::Listening {
                spec.practice.srs.listening_weight
            } else {
                1.0
            };
            if s.box_idx >= 5 {
                // Box-5 recheck passed: stay, schedule next recheck.
                s.progress = 0.0;
                s.due_at = now + i64::from(spec.recheck_days()) * SECONDS_PER_DAY;
            } else {
                s.progress += weight;
                while s.progress >= 1.0 && s.box_idx < 5 {
                    s.progress -= 1.0;
                    s.box_idx += 1;
                }
                if s.box_idx >= 5 {
                    s.progress = 0.0;
                }
                s.due_at = now + i64::from(spec.interval_days(s.box_idx)) * SECONDS_PER_DAY;
            }
        }
        Outcome::Wrong => {
            s.err += 1;
            s.box_idx = 1;
            s.progress = 0.0;
            // 盒 1 当天:立即可重练.
            s.due_at = now;
        }
        Outcome::MarkMastered => {
            s.box_idx = 5;
            s.progress = 0.0;
            s.due_at = now + i64::from(spec.recheck_days()) * SECONDS_PER_DAY;
        }
        Outcome::MarkUnfamiliar => {
            s.marked_unfamiliar = true;
            s.box_idx = 1;
            s.progress = 0.0;
            s.due_at = now;
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::LevelSpec;

    fn spec() -> LevelSpec {
        LevelSpec::from_yaml(crate::spec::tests_support::L3_YAML).unwrap()
    }

    const NOW: i64 = 1_700_000_000;

    #[test]
    fn correct_advances_one_box() {
        let s0 = SrsState::new(NOW);
        let s1 = apply_outcome(
            s0,
            Outcome::Correct { seen_answer: false },
            Mode::Typing,
            &spec(),
            NOW,
        );
        assert_eq!(s1.box_idx, 2);
        assert_eq!(s1.due_at, NOW + SECONDS_PER_DAY); // box 2 = +1d
        let s2 = apply_outcome(
            s1,
            Outcome::Correct { seen_answer: false },
            Mode::Typing,
            &spec(),
            NOW,
        );
        assert_eq!(s2.box_idx, 3);
        assert_eq!(s2.due_at, NOW + 3 * SECONDS_PER_DAY);
    }

    #[test]
    fn listening_weight_accelerates() {
        // Two listening corrects at weight 1.5 = 3.0 progress = 3 boxes.
        let s0 = SrsState::new(NOW);
        let s1 = apply_outcome(
            s0,
            Outcome::Correct { seen_answer: false },
            Mode::Listening,
            &spec(),
            NOW,
        );
        assert_eq!(s1.box_idx, 2);
        assert!((s1.progress - 0.5).abs() < 1e-6);
        let s2 = apply_outcome(
            s1,
            Outcome::Correct { seen_answer: false },
            Mode::Listening,
            &spec(),
            NOW,
        );
        assert_eq!(s2.box_idx, 4);
        assert!(s2.is_mastered());
    }

    #[test]
    fn wrong_resets_to_box1_due_now() {
        let mut s = SrsState::new(NOW);
        s.box_idx = 4;
        let s1 = apply_outcome(s, Outcome::Wrong, Mode::Typing, &spec(), NOW + 10);
        assert_eq!(s1.box_idx, 1);
        assert_eq!(s1.err, 1);
        assert_eq!(s1.due_at, NOW + 10);
        assert!(s1.is_due(NOW + 10));
    }

    #[test]
    fn mark_mastered_jumps_to_box5() {
        let s = SrsState::new(NOW);
        let s1 = apply_outcome(s, Outcome::MarkMastered, Mode::Typing, &spec(), NOW);
        assert_eq!(s1.box_idx, 5);
        assert_eq!(s1.due_at, NOW + 30 * SECONDS_PER_DAY);
    }

    #[test]
    fn box5_recheck_stays_in_box5() {
        let s = SrsState {
            box_idx: 5,
            ..SrsState::new(NOW)
        };
        let s1 = apply_outcome(
            s,
            Outcome::Correct { seen_answer: false },
            Mode::Typing,
            &spec(),
            NOW,
        );
        assert_eq!(s1.box_idx, 5);
        assert_eq!(s1.due_at, NOW + 30 * SECONDS_PER_DAY);
    }

    #[test]
    fn unfamiliar_enters_wrongbook() {
        let s = SrsState {
            box_idx: 3,
            ..SrsState::new(NOW)
        };
        let s1 = apply_outcome(s, Outcome::MarkUnfamiliar, Mode::Typing, &spec(), NOW);
        assert!(s1.qualifies_for_wrongbook());
        assert_eq!(s1.box_idx, 1);
    }

    #[test]
    fn two_wrongs_enter_wrongbook() {
        let s = SrsState::new(NOW);
        let s1 = apply_outcome(s, Outcome::Wrong, Mode::Typing, &spec(), NOW);
        assert!(!s1.qualifies_for_wrongbook());
        let s2 = apply_outcome(s1, Outcome::Wrong, Mode::Typing, &spec(), NOW);
        assert!(s2.qualifies_for_wrongbook());
    }

    #[test]
    fn seen_answer_recorded_but_still_advances() {
        let s = SrsState::new(NOW);
        let s1 = apply_outcome(
            s,
            Outcome::Correct { seen_answer: true },
            Mode::Typing,
            &spec(),
            NOW,
        );
        assert!(s1.seen_answer);
        assert_eq!(s1.box_idx, 2);
    }
}
