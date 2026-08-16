//! # sf-pipeline — unified generation pipeline (spec §7.4)
//!
//! One pipeline, two profiles:
//! * **factory** — builds the factory sentence bank on the vendor machine
//!   (strict validation, over-level sentences are re-leveled, gold regression);
//! * **user** — the in-app 生成工坊 (lenient validation, over-level sentences
//!   are discarded-but-recoverable).
//!
//! The pipeline is deliberately split from transport: sf-llm streams text in,
//! this crate turns text into *validated* [`sf_core::Sentence`] rows. Nothing
//! reaches a database without passing [`validate`].

pub mod lexicon;
pub mod parse;
pub mod prompt;
pub mod seed;
pub mod simhash;
pub mod triage;
pub mod validate;

#[cfg(feature = "store")]
pub mod store;

pub use parse::{DraftSentence, DraftWord, extract_json_array, parse_drafts};
pub use prompt::{PromptParts, build_prompt};
pub use simhash::{hamming_distance, simhash64};
pub use triage::{GenProfile, TriageOutcome, triage};
pub use validate::{ValidationIssue, ValidationReport, Validator};
