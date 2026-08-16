//! # sf-wasm — sf-core exposed to the web trial (spec §7.9)
//!
//! The ABI is JSON strings in / JSON strings out: trivially stable across
//! wasm-bindgen versions, and the TS side shares the exact serde shapes the
//! desktop Tauri commands use — 双端逐比特一致 (§7.3).
//!
//! Errors are returned as thrown JS strings (wasm-bindgen converts
//! `Result<_, String>`).

use sf_core::{JudgePolicy, LevelSpec, LogRow, SrsState};
use wasm_bindgen::prelude::*;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Parse + validate a LevelSpec YAML document; returns it as JSON.
#[wasm_bindgen]
pub fn parse_level_spec(yaml: &str) -> Result<String, String> {
    let spec = LevelSpec::from_yaml(yaml).map_err(err)?;
    serde_json::to_string(&spec).map_err(err)
}

/// `due`: JSON `[[sentence_id, SrsState], …]`; `new_pool`: JSON `[id, …]`;
/// `spec`: JSON LevelSpec. Returns a JSON `Session`.
#[wasm_bindgen]
pub fn build_session(
    due_json: &str,
    new_pool_json: &str,
    spec_json: &str,
    now: i64,
    seed: u64,
    daily_new_override: Option<u32>,
) -> Result<String, String> {
    let due: Vec<(i64, SrsState)> = serde_json::from_str(due_json).map_err(err)?;
    let new_pool: Vec<i64> = serde_json::from_str(new_pool_json).map_err(err)?;
    let spec: LevelSpec = serde_json::from_str(spec_json).map_err(err)?;
    let session = sf_core::build_session(&due, &new_pool, &spec, now, seed, daily_new_override);
    serde_json::to_string(&session).map_err(err)
}

/// `state`: JSON SrsState; `outcome`: JSON Outcome; `mode`: JSON Mode.
#[wasm_bindgen]
pub fn apply_outcome(
    state_json: &str,
    outcome_json: &str,
    mode_json: &str,
    spec_json: &str,
    now: i64,
) -> Result<String, String> {
    let state: SrsState = serde_json::from_str(state_json).map_err(err)?;
    let outcome = serde_json::from_str(outcome_json).map_err(err)?;
    let mode = serde_json::from_str(mode_json).map_err(err)?;
    let spec: LevelSpec = serde_json::from_str(spec_json).map_err(err)?;
    let next = sf_core::apply_outcome(state, outcome, mode, &spec, now);
    serde_json::to_string(&next).map_err(err)
}

/// Fresh SRS state for a sentence first seen at `now`.
#[wasm_bindgen]
pub fn new_srs_state(now: i64) -> Result<String, String> {
    serde_json::to_string(&SrsState::new(now)).map_err(err)
}

/// `targets`: JSON `["word", …]`; `policy`: JSON JudgePolicy (or null for
/// defaults). Returns a JSON Verdict.
#[wasm_bindgen]
pub fn judge(
    input: &str,
    targets_json: &str,
    policy_json: Option<String>,
) -> Result<String, String> {
    let targets: Vec<String> = serde_json::from_str(targets_json).map_err(err)?;
    let policy: JudgePolicy = match policy_json {
        Some(p) => serde_json::from_str(&p).map_err(err)?,
        None => JudgePolicy::default(),
    };
    let refs: Vec<&str> = targets.iter().map(String::as_str).collect();
    let verdict = sf_core::judge(input, &refs, &policy);
    serde_json::to_string(&verdict).map_err(err)
}

/// `logs`: JSON `[LogRow, …]`. Returns a JSON StatsSummary.
#[wasm_bindgen]
pub fn fold_stats(logs_json: &str, tz_offset_secs: i32) -> Result<String, String> {
    let logs: Vec<LogRow> = serde_json::from_str(logs_json).map_err(err)?;
    let summary = sf_core::fold_stats(logs, tz_offset_secs);
    serde_json::to_string(&summary).map_err(err)
}

#[cfg(test)]
mod tests {
    // The bindings are thin shims; these tests exercise the JSON boundary on
    // the native target (the wasm build is CI's `wasm32-unknown-unknown` job).
    use super::*;

    const SPEC_YAML: &str = r#"
id: L1
cefr: "A1"
vocab_band: 500
max_words: 8
grammar_whitelist: [be_present]
can_do: ["打招呼"]
practice:
  flow: reorder_then_typing
  review_listening_ratio: 0.0
  dictation_min_box: 0
  hints: { ipa: always, first_letter: true, zh_hideable: false }
  judge: { strict: false }
  srs:
    daily_new_default: 10
    daily_new_range: [5, 50]
    review_cap: 60
    box_intervals_days: [1, 2, 4, 7]
    box5_recheck_days: 12
    listening_weight: 1.5
"#;

    #[test]
    fn full_json_roundtrip() {
        let spec_json = parse_level_spec(SPEC_YAML).unwrap();
        let state = new_srs_state(1000).unwrap();
        let session = build_session("[]", "[1,2,3]", &spec_json, 1000, 42, None).unwrap();
        assert!(session.contains("\"sentence_id\":1"));
        let next = apply_outcome(
            &state,
            r#"{"kind":"correct","seen_answer":false}"#,
            r#""typing""#,
            &spec_json,
            1000,
        )
        .unwrap();
        assert!(next.contains("\"box_idx\":2"));
        let verdict = judge("i am fine", r#"["I","am","fine"]"#, None).unwrap();
        assert!(verdict.contains("\"correct\":true"));
        let stats = fold_stats("[]", 0).unwrap();
        assert!(stats.contains("\"streak_days\":0"));
    }
}
