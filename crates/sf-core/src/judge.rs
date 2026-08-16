//! Whole-sentence judging (spec §4.1).
//!
//! Typing/reorder modes submit exactly the target word count, but 默写 is free
//! input, so the judge aligns input words to target words with an LCS diff and
//! reports per-word verdicts: 对绿、错红删除线附正确词、漏词灰占位 (§4.1).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct JudgePolicy {
    /// 大小写不敏感(默认 true,§4.1).
    pub case_insensitive: bool,
    /// 忽略输入中的标点(句末标点直显不输入,输入里出现的标点剥离).
    pub ignore_punct: bool,
}

impl Default for JudgePolicy {
    fn default() -> Self {
        Self {
            case_insensitive: true,
            ignore_punct: true,
        }
    }
}

/// Verdict for one aligned position.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WordVerdict {
    /// 对 — 绿色.
    Correct { word: String },
    /// 错 — 红删除线 got,附正确词 expected.
    Wrong { expected: String, got: String },
    /// 漏词 — 灰占位 expected.
    Missing { expected: String },
    /// 多打的词.
    Extra { got: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verdict {
    pub words: Vec<WordVerdict>,
    /// Whole sentence correct (no wrong/missing/extra).
    pub correct: bool,
    /// Number of non-correct positions.
    pub errors: u32,
}

fn normalize(word: &str, policy: &JudgePolicy) -> String {
    let mut s: String = if policy.ignore_punct {
        word.chars()
            .filter(|c| c.is_ascii_alphabetic() || *c == '\'')
            .collect()
    } else {
        word.to_string()
    };
    if policy.case_insensitive {
        s = s.to_lowercase();
    }
    s
}

fn split_words(input: &str) -> Vec<&str> {
    input.split_whitespace().filter(|w| !w.is_empty()).collect()
}

/// Judge free-form `input` against `target_words`.
///
/// The alignment maximises matched words (classic LCS), then walks both
/// sequences emitting `Wrong` where one unmatched input word faces one
/// unmatched target word, `Missing`/`Extra` otherwise.
pub fn judge(input: &str, target_words: &[&str], policy: &JudgePolicy) -> Verdict {
    let input_words = split_words(input);
    let a: Vec<String> = input_words.iter().map(|w| normalize(w, policy)).collect();
    let b: Vec<String> = target_words.iter().map(|w| normalize(w, policy)).collect();

    // LCS table (sentences are ≤ ~20 words, O(n·m) is trivial).
    let (n, m) = (a.len(), b.len());
    let mut lcs = vec![vec![0u16; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }

    let mut words = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    // Buffers of unmatched runs between matches; paired up as Wrong.
    let mut pend_in: Vec<usize> = Vec::new();
    let mut pend_tg: Vec<usize> = Vec::new();

    let flush =
        |words: &mut Vec<WordVerdict>, pend_in: &mut Vec<usize>, pend_tg: &mut Vec<usize>| {
            let pairs = pend_in.len().min(pend_tg.len());
            for k in 0..pairs {
                words.push(WordVerdict::Wrong {
                    expected: target_words[pend_tg[k]].to_string(),
                    got: input_words[pend_in[k]].to_string(),
                });
            }
            for &tg in pend_tg.iter().skip(pairs) {
                words.push(WordVerdict::Missing {
                    expected: target_words[tg].to_string(),
                });
            }
            for &iw in pend_in.iter().skip(pairs) {
                words.push(WordVerdict::Extra {
                    got: input_words[iw].to_string(),
                });
            }
            pend_in.clear();
            pend_tg.clear();
        };

    while i < n || j < m {
        if i < n && j < m && a[i] == b[j] {
            flush(&mut words, &mut pend_in, &mut pend_tg);
            words.push(WordVerdict::Correct {
                word: target_words[j].to_string(),
            });
            i += 1;
            j += 1;
        } else if j < m && (i >= n || lcs[i][j + 1] >= lcs.get(i + 1).map_or(0, |r| r[j])) {
            pend_tg.push(j);
            j += 1;
        } else {
            pend_in.push(i);
            i += 1;
        }
    }
    flush(&mut words, &mut pend_in, &mut pend_tg);

    let errors = words
        .iter()
        .filter(|w| !matches!(w, WordVerdict::Correct { .. }))
        .count() as u32;
    Verdict {
        correct: errors == 0,
        words,
        errors,
    }
}

/// Single-character check for strict typing mode (错字不上屏,§4.1):
/// is `ch` the next expected letter of `target_word` at `typed_len`?
pub fn accepts_char(target_word: &str, typed_len: usize, ch: char, policy: &JudgePolicy) -> bool {
    let expected = match target_word.chars().nth(typed_len) {
        Some(c) => c,
        None => return false,
    };
    if policy.case_insensitive {
        expected.to_lowercase().eq(ch.to_lowercase())
    } else {
        expected == ch
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p() -> JudgePolicy {
        JudgePolicy::default()
    }

    #[test]
    fn exact_match_is_correct() {
        let v = judge("I am fine", &["I", "am", "fine"], &p());
        assert!(v.correct);
        assert_eq!(v.errors, 0);
        assert_eq!(v.words.len(), 3);
    }

    #[test]
    fn case_insensitive_by_default() {
        let v = judge("i AM Fine", &["I", "am", "fine"], &p());
        assert!(v.correct);
    }

    #[test]
    fn punctuation_in_input_is_ignored() {
        let v = judge("I am fine.", &["I", "am", "fine"], &p());
        assert!(v.correct);
    }

    #[test]
    fn wrong_word_pairs_up() {
        let v = judge("I am find", &["I", "am", "fine"], &p());
        assert!(!v.correct);
        assert_eq!(v.errors, 1);
        assert!(matches!(
            &v.words[2],
            WordVerdict::Wrong { expected, got } if expected == "fine" && got == "find"
        ));
    }

    #[test]
    fn missing_word_reported() {
        let v = judge("I fine", &["I", "am", "fine"], &p());
        assert_eq!(v.errors, 1);
        assert!(
            v.words
                .iter()
                .any(|w| matches!(w, WordVerdict::Missing { expected } if expected == "am"))
        );
    }

    #[test]
    fn extra_word_reported() {
        let v = judge("I am very fine", &["I", "am", "fine"], &p());
        assert_eq!(v.errors, 1);
        assert!(
            v.words
                .iter()
                .any(|w| matches!(w, WordVerdict::Extra { got } if got == "very"))
        );
    }

    #[test]
    fn apostrophes_survive_normalization() {
        let v = judge("don't worry", &["don't", "worry"], &p());
        assert!(v.correct);
    }

    #[test]
    fn empty_input_is_all_missing() {
        let v = judge("", &["I", "am", "fine"], &p());
        assert_eq!(v.errors, 3);
        assert!(
            v.words
                .iter()
                .all(|w| matches!(w, WordVerdict::Missing { .. }))
        );
    }

    #[test]
    fn accepts_char_strict_mode() {
        assert!(accepts_char("fine", 0, 'f', &p()));
        assert!(accepts_char("fine", 0, 'F', &p()));
        assert!(!accepts_char("fine", 0, 'x', &p()));
        assert!(accepts_char("fine", 3, 'e', &p()));
        assert!(!accepts_char("fine", 4, 'e', &p())); // word already full
    }
}
