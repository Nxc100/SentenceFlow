//! Daily-session builder (spec §4.5 / §4.9).
//!
//! 每日队列 = 到期复习优先(上限 review_cap)+ 新句补足;复习与新句混排。
//! Mode assignment is data-driven from the LevelSpec practice section:
//! * review items convert to 听打 at `review_listening_ratio`,
//! * items in box ≥ `dictation_min_box` may convert to 默写 (when enabled),
//! * new items use the spec flow (重组先行 is a per-item flag for the UI).
//!
//! Everything is deterministic for a given `(inputs, now, seed)` triple.

use crate::rng::SplitMix64;
use crate::spec::{FlowKind, LevelSpec};
use crate::srs::{Mode, SrsState};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionItem {
    pub sentence_id: i64,
    pub mode: Mode,
    /// True for 到期复习 items (UI shows the small dot hint, §4.5).
    pub is_review: bool,
    /// UI should run 拆句重组 before typing (L1–L2 first-pass flow, §4.9).
    pub reorder_first: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub items: Vec<SessionItem>,
    /// How many review items were due but cut by `review_cap`.
    pub overflow_reviews: u32,
}

impl Session {
    pub fn review_count(&self) -> usize {
        self.items.iter().filter(|i| i.is_review).count()
    }

    pub fn new_count(&self) -> usize {
        self.items.len() - self.review_count()
    }
}

/// Build today's queue.
///
/// * `due` — candidate review sentences with their SRS state (any order; the
///   builder filters by `due_at <= now` itself).
/// * `new_pool` — unseen sentences in curriculum order.
/// * `daily_new_override` — user setting, clamped into the spec range.
pub fn build_session(
    due: &[(i64, SrsState)],
    new_pool: &[i64],
    spec: &LevelSpec,
    now: i64,
    seed: u64,
    daily_new_override: Option<u32>,
) -> Session {
    let mut rng = SplitMix64::new(seed);

    // 1. Reviews: most overdue first, capped.
    let mut due_now: Vec<&(i64, SrsState)> = due.iter().filter(|(_, s)| s.is_due(now)).collect();
    due_now.sort_by_key(|(id, s)| (s.due_at, *id));
    let cap = spec.practice.srs.review_cap as usize;
    let overflow_reviews = due_now.len().saturating_sub(cap) as u32;
    due_now.truncate(cap);

    let mut items: Vec<SessionItem> = due_now
        .iter()
        .map(|(id, state)| SessionItem {
            sentence_id: *id,
            mode: pick_review_mode(state, spec, &mut rng),
            is_review: true,
            reorder_first: false,
        })
        .collect();

    // 2. New sentences fill up to the daily-new quota.
    let quota = spec.effective_daily_new(daily_new_override) as usize;
    let reorder_first = spec.practice.flow == FlowKind::ReorderThenTyping;
    items.extend(new_pool.iter().take(quota).map(|id| SessionItem {
        sentence_id: *id,
        mode: Mode::Typing,
        is_review: false,
        reorder_first,
    }));

    // 3. 混排 — deterministic shuffle of the combined queue.
    rng.shuffle(&mut items);

    Session {
        items,
        overflow_reviews,
    }
}

fn pick_review_mode(state: &SrsState, spec: &LevelSpec, rng: &mut SplitMix64) -> Mode {
    let p = &spec.practice;
    // 默写 first: it is the rarer, higher-value conversion (盒 ≥3, L5–L6).
    if p.dictation_min_box > 0 && state.box_idx >= p.dictation_min_box {
        // Half of eligible reviews go to dictation, the rest fall through to
        // the listening/typing split so a session mixes all unlocked modes.
        if rng.next_f64() < 0.5 {
            return Mode::Dictation;
        }
    }
    if p.review_listening_ratio > 0.0 && rng.next_f64() < f64::from(p.review_listening_ratio) {
        return Mode::Listening;
    }
    Mode::Typing
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::LevelSpec;
    use crate::srs::SECONDS_PER_DAY;

    const NOW: i64 = 1_700_000_000;

    fn spec() -> LevelSpec {
        LevelSpec::from_yaml(crate::spec::tests_support::L3_YAML).unwrap()
    }

    fn due_state(due_at: i64) -> SrsState {
        SrsState {
            due_at,
            ..SrsState::new(0)
        }
    }

    #[test]
    fn reviews_before_cap_new_fill_quota() {
        let due: Vec<(i64, SrsState)> = (0..10).map(|i| (i, due_state(NOW - i * 60))).collect();
        let pool: Vec<i64> = (100..200).collect();
        let s = build_session(&due, &pool, &spec(), NOW, 1, None);
        assert_eq!(s.review_count(), 10);
        assert_eq!(s.new_count(), 20); // L3 default daily_new = 20
        assert_eq!(s.overflow_reviews, 0);
    }

    #[test]
    fn respects_review_cap_and_reports_overflow() {
        let due: Vec<(i64, SrsState)> = (0..80).map(|i| (i, due_state(NOW - i))).collect();
        let s = build_session(&due, &[], &spec(), NOW, 1, None);
        assert_eq!(s.review_count(), 60);
        assert_eq!(s.overflow_reviews, 20);
    }

    #[test]
    fn not_due_items_are_excluded() {
        let due = vec![
            (1, due_state(NOW + SECONDS_PER_DAY)),
            (2, due_state(NOW - 1)),
        ];
        let s = build_session(&due, &[], &spec(), NOW, 1, None);
        assert_eq!(s.items.len(), 1);
        assert_eq!(s.items[0].sentence_id, 2);
    }

    #[test]
    fn deterministic_for_same_seed() {
        let due: Vec<(i64, SrsState)> = (0..30).map(|i| (i, due_state(NOW - i))).collect();
        let pool: Vec<i64> = (100..160).collect();
        let a = build_session(&due, &pool, &spec(), NOW, 42, Some(15));
        let b = build_session(&due, &pool, &spec(), NOW, 42, Some(15));
        assert_eq!(a, b);
        let c = build_session(&due, &pool, &spec(), NOW, 43, Some(15));
        assert_ne!(a.items, c.items); // different seed ⇒ different order
    }

    #[test]
    fn listening_ratio_applies_to_reviews() {
        let due: Vec<(i64, SrsState)> = (0..60).map(|i| (i, due_state(NOW - i))).collect();
        let s = build_session(&due, &[], &spec(), NOW, 7, None);
        let listening = s.items.iter().filter(|i| i.mode == Mode::Listening).count();
        // ratio 0.3 over 60 items — allow a broad band, but it must be used.
        assert!(listening > 5 && listening < 35, "listening = {listening}");
        // L3 spec has dictation disabled.
        assert!(s.items.iter().all(|i| i.mode != Mode::Dictation));
    }

    #[test]
    fn dictation_only_from_min_box() {
        let mut spec = spec();
        spec.practice.dictation_min_box = 3;
        let low: Vec<(i64, SrsState)> = (0..40)
            .map(|i| {
                (
                    i,
                    SrsState {
                        box_idx: 2,
                        ..due_state(NOW - i)
                    },
                )
            })
            .collect();
        let s = build_session(&low, &[], &spec, NOW, 3, None);
        assert!(s.items.iter().all(|i| i.mode != Mode::Dictation));

        let high: Vec<(i64, SrsState)> = (0..40)
            .map(|i| {
                (
                    i,
                    SrsState {
                        box_idx: 4,
                        ..due_state(NOW - i)
                    },
                )
            })
            .collect();
        let s = build_session(&high, &[], &spec, NOW, 3, None);
        assert!(s.items.iter().any(|i| i.mode == Mode::Dictation));
    }

    #[test]
    fn reorder_first_flag_follows_flow() {
        let mut spec = spec();
        spec.practice.flow = FlowKind::ReorderThenTyping;
        let pool: Vec<i64> = (0..10).collect();
        let s = build_session(&[], &pool, &spec, NOW, 1, Some(10));
        assert!(s.items.iter().all(|i| i.reorder_first));
    }
}
