//! Post-validation triage — what happens to each draft (spec §7.4 分诊).
//!
//! | verdict      | factory 档                      | user 档(生成工坊)          |
//! |--------------|---------------------------------|-----------------------------|
//! | Pass         | accept                          | accept                      |
//! | NeedsRepair  | repair call (仅传差异)           | repair call                 |
//! | OverLevel    | relevel to the fitting level    | discard, 可捞回             |
//! | Duplicate    | discard                         | discard, 可捞回             |
//! | Broken       | discard (回灌反例库)             | discard                     |

use crate::validate::{ValidationIssue, ValidationReport, VerdictKind};
use serde::{Deserialize, Serialize};
use sf_core::Sentence;
use sf_core::sentence::LevelId;
use sf_core::spec::LevelSpec;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GenProfile {
    /// 开发侧出厂库生产:严格,越级改级.
    Factory,
    /// 生成工坊:宽松,越级即弃可捞回.
    User,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum TriageOutcome {
    /// Store the sentence.
    Accept { sentence: Sentence },
    /// Ask the model to patch just these issues (修补调用).
    Repair {
        sentence: Sentence,
        issues: Vec<ValidationIssue>,
    },
    /// Factory only: the sentence belongs to a different level.
    Relevel {
        sentence: Sentence,
        new_level: LevelId,
    },
    /// Drop. `recoverable` backs the 丢弃句折叠可捞回 UI (§4.4).
    Discard {
        recoverable: Option<Sentence>,
        reason: String,
    },
}

/// Decide the fate of one validated draft.
///
/// `all_specs` must contain every level (used to find a fitting level when
/// releveling); it may be in any order.
pub fn triage(
    report: ValidationReport,
    profile: GenProfile,
    all_specs: &[LevelSpec],
) -> TriageOutcome {
    let reason = || {
        report
            .issues
            .iter()
            .filter(|i| i.severity() != crate::validate::Severity::AutoFixed)
            .map(ValidationIssue::zh_reason)
            .collect::<Vec<_>>()
            .join("；")
    };

    match report.verdict {
        VerdictKind::Pass => TriageOutcome::Accept {
            sentence: report.sentence.expect("Pass always carries a sentence"),
        },
        VerdictKind::NeedsRepair => TriageOutcome::Repair {
            sentence: report
                .sentence
                .expect("NeedsRepair always carries a sentence"),
            issues: report
                .issues
                .iter()
                .filter(|i| i.severity() == crate::validate::Severity::Repairable)
                .cloned()
                .collect(),
        },
        VerdictKind::OverLevel => {
            let sentence = report
                .sentence
                .expect("OverLevel always carries a sentence");
            match profile {
                GenProfile::Factory => match fitting_level(&report.issues, &sentence, all_specs) {
                    Some(new_level) => TriageOutcome::Relevel {
                        sentence,
                        new_level,
                    },
                    None => TriageOutcome::Discard {
                        recoverable: Some(sentence),
                        reason: reason(),
                    },
                },
                GenProfile::User => TriageOutcome::Discard {
                    recoverable: Some(sentence),
                    reason: reason(),
                },
            }
        }
        VerdictKind::Duplicate => TriageOutcome::Discard {
            recoverable: match profile {
                GenProfile::Factory => None,
                GenProfile::User => report.sentence,
            },
            reason: reason(),
        },
        VerdictKind::Broken => TriageOutcome::Discard {
            recoverable: None,
            reason: reason(),
        },
    }
}

/// Smallest level whose band and length limits absorb all level issues.
fn fitting_level(
    issues: &[ValidationIssue],
    sentence: &Sentence,
    all_specs: &[LevelSpec],
) -> Option<LevelId> {
    let max_band = issues
        .iter()
        .filter_map(|i| match i {
            ValidationIssue::OverLevel { band, .. } => Some(*band),
            _ => None,
        })
        .max();
    // Unknown words can never be resolved by releveling.
    if issues
        .iter()
        .any(|i| matches!(i, ValidationIssue::UnknownWord { .. }))
    {
        return None;
    }
    let len = sentence.words.len() as u32;

    let mut specs: Vec<&LevelSpec> = all_specs.iter().collect();
    specs.sort_by_key(|s| s.id);
    specs
        .into_iter()
        .find(|s| {
            s.id > sentence.level
                && s.max_words >= len
                && max_band.is_none_or(|b| s.vocab_band == 0 || s.vocab_band >= b)
        })
        .map(|s| s.id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexicon::Lexicon;
    use crate::parse::{DraftChunk, DraftSentence, DraftWord};
    use crate::validate::{DedupeIndex, Validator};

    fn spec_yaml(id: &str, band: u32, max_words: u32) -> String {
        format!(
            r#"
id: {id}
cefr: "A1"
vocab_band: {band}
max_words: {max_words}
grammar_whitelist: []
can_do: []
practice:
  flow: typing
  review_listening_ratio: 0.0
  dictation_min_box: 0
  hints: {{ ipa: always, first_letter: false, zh_hideable: false }}
  judge: {{ strict: true }}
  srs:
    daily_new_default: 10
    daily_new_range: [5, 50]
    review_cap: 60
    box_intervals_days: [1, 2, 4, 7]
    box5_recheck_days: 12
    listening_weight: 1.5
"#
        )
    }

    fn all_specs() -> Vec<LevelSpec> {
        vec![
            LevelSpec::from_yaml(&spec_yaml("L1", 500, 8)).unwrap(),
            LevelSpec::from_yaml(&spec_yaml("L3", 1500, 12)).unwrap(),
            LevelSpec::from_yaml(&spec_yaml("L5", 2800, 16)).unwrap(),
        ]
    }

    fn lexicon() -> Lexicon {
        Lexicon::from_tsv(
            "i\t2\taɪ\taɪ\t我\nbe\t1\tbi\tbi\t是\npassport\t2200\tpɑːspɔːt\tpæspɔːrt\t护照\n",
        )
        .unwrap()
    }

    fn over_level_report() -> ValidationReport {
        let specs = all_specs();
        let lex = lexicon();
        let draft = DraftSentence {
            en: "I am passport.".into(),
            zh: "护照句。".into(),
            pattern: String::new(),
            words: vec![
                DraftWord {
                    w: "I".into(),
                    ipa: "aɪ".into(),
                    pos: "pron".into(),
                },
                DraftWord {
                    w: "am".into(),
                    ipa: "æm".into(),
                    pos: "aux".into(),
                },
                DraftWord {
                    w: "passport".into(),
                    ipa: "pɑːspɔːt".into(),
                    pos: "n".into(),
                },
            ],
            chunks: vec![
                DraftChunk {
                    r: "subj".into(),
                    i: vec![0],
                },
                DraftChunk {
                    r: "link".into(),
                    i: vec![1],
                },
                DraftChunk {
                    r: "comp".into(),
                    i: vec![2],
                },
            ],
            note: String::new(),
            speaker: String::new(),
        };
        Validator::new(&specs[0], &lex).validate(&draft, "s", "f", &DedupeIndex::default())
    }

    #[test]
    fn factory_relevels_over_level() {
        // "passport" is band 2200: too high for L1 (500) and L3 (1500), so
        // the smallest fitting level is L5 (2800).
        let out = triage(over_level_report(), GenProfile::Factory, &all_specs());
        match out {
            TriageOutcome::Relevel { new_level, .. } => assert_eq!(new_level, LevelId::L5),
            other => panic!("expected Relevel, got {other:?}"),
        }
    }

    #[test]
    fn user_discards_over_level_recoverably() {
        let out = triage(over_level_report(), GenProfile::User, &all_specs());
        match out {
            TriageOutcome::Discard {
                recoverable,
                reason,
            } => {
                assert!(recoverable.is_some());
                assert!(reason.contains("passport"), "reason = {reason}");
            }
            other => panic!("expected Discard, got {other:?}"),
        }
    }
}
