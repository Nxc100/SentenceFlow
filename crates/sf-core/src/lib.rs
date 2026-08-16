//! # sf-core — SentenceFlow pure-logic core
//!
//! Everything in this crate is a pure function over plain data:
//! no filesystem, no network, no clocks, no OS randomness. Time is always an
//! explicit `now: i64` (unix seconds) parameter and randomness is an explicit
//! `seed: u64`, so the desktop app (native) and the web trial (wasm) produce
//! bit-identical behaviour from identical inputs (spec §7.3).
//!
//! Layering (spec §4.9): the [`spec::LevelSpec`] is the single source of truth
//! for *how a level is practised*; this crate only interprets it and contains
//! no per-level hardcoding.

pub mod judge;
pub mod rng;
pub mod sentence;
pub mod session;
pub mod spec;
pub mod srs;
pub mod stats;

pub use judge::{JudgePolicy, Verdict, WordVerdict, judge};
pub use sentence::{Chunk, LevelId, PosTag, RoleTag, Sentence, Word};
pub use session::{Session, SessionItem, build_session};
pub use spec::{FlowKind, HintVisibility, LevelSpec, PracticeSpec, SrsSpec};
pub use srs::{Mode, Outcome, SrsState, apply_outcome};
pub use stats::{DayStats, ErrorTag, LogResult, LogRow, StatsSummary, fold_stats};
