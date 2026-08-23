//! Deterministic validation — the contract layer of the pipeline (spec §7.4).
//!
//! Whatever the model or channel does, nothing enters a database without
//! passing here (the third of the three agent constraints, §3.4). All checks
//! are pure and cost zero tokens:
//!
//! * structure: en/zh present, words tokenization matches `en`, POS/role tags
//!   inside the closed enums, chunks cover every word exactly once;
//! * level: NGSL band 越级, unknown words, sentence length vs the spec;
//! * phonetics: IPA charset; lemma-exact IPA is *overwritten* from the lexicon
//!   (音标漂移归零 — recorded as an auto-fix, not a failure);
//! * dedupe: simhash near-duplicate against already-accepted fingerprints.

pub use crate::dedupe::DedupeIndex;
use crate::dedupe::{DedupeKey, NEAR_DUP_MIN_SIMILARITY};
use crate::lexicon::Lexicon;
use crate::parse::DraftSentence;
use crate::simhash::simhash64;
use serde::{Deserialize, Serialize};
use sf_core::spec::LevelSpec;
use sf_core::{Chunk, PosTag, RoleTag, Sentence, Word};

/// Characters permitted in IPA transcriptions (English phoneme inventory).
///
/// 公开是为了让 `sf gold run` 能拿它反过来体检词表自己的 IPA 列:
/// 词典对账会把词表里的音标**覆写**进句子,词表里混进一个非法字符,
/// 生成时就会判 BadIpaChars → 触发修补调用(一次完整请求),
/// 而且每次生成到这个词都会再犯一遍。
pub const IPA_ALLOWED: &str = "abcdefghijklmnopqrstuvwxyzæɑɒɔəɜɛɪʊʌŋʃʒθðɡːˈˌ.'’ ";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ValidationIssue {
    // ---- fatal: structurally broken, can never be shown to a learner ----
    EmptyEnglish,
    EmptyChinese,
    NoWords,
    /// `en` tokenization disagrees with the `words` array.
    TokenMismatch {
        expected: Vec<String>,
        got: Vec<String>,
    },
    UnknownPosTag {
        word_index: usize,
        tag: String,
    },
    UnknownRoleTag {
        chunk_index: usize,
        tag: String,
    },
    ChunkIndexOutOfRange {
        chunk_index: usize,
        index: usize,
    },
    ChunkOverlap {
        word_index: usize,
    },
    ChunkGap {
        word_indices: Vec<usize>,
    },
    // ---- repairable: needs a patch call (仅传差异,§7.4) ----
    BadIpaChars {
        word_index: usize,
        ipa: String,
    },
    MissingIpa {
        word_index: usize,
    },
    /// 确定性语法检查抓到的错(见 [`crate::grammar`])。归为可修补而非丢弃:
    /// 这几类都是模型一次修补就能改对的小错,句子内容本身是好的。
    Grammar {
        problem: crate::grammar::GrammarProblem,
    },
    // ---- level: sentence is fine, just not for this level ----
    OverLevel {
        word: String,
        band: u32,
        allowed: u32,
    },
    UnknownWord {
        word: String,
    },
    TooLong {
        len: usize,
        max: u32,
    },
    // ---- dedupe ----
    NearDuplicate {
        /// 与场景内最像的一句的相似度(见 [`crate::dedupe`] 的标定)。
        similarity: f64,
    },
    // ---- auto-fixes actually applied (informational) ----
    IpaReconciled {
        word_index: usize,
        from: String,
        to: String,
    },
}

impl ValidationIssue {
    pub fn severity(&self) -> Severity {
        use ValidationIssue::*;
        match self {
            EmptyEnglish
            | EmptyChinese
            | NoWords
            | TokenMismatch { .. }
            | UnknownPosTag { .. }
            | UnknownRoleTag { .. }
            | ChunkIndexOutOfRange { .. }
            | ChunkOverlap { .. }
            | ChunkGap { .. } => Severity::Fatal,
            BadIpaChars { .. } | MissingIpa { .. } | Grammar { .. } => Severity::Repairable,
            OverLevel { .. } | UnknownWord { .. } | TooLong { .. } => Severity::Level,
            NearDuplicate { .. } => Severity::Duplicate,
            IpaReconciled { .. } => Severity::AutoFixed,
        }
    }

    /// One-line human explanation for the ✕ badge's expandable reason (§5.5).
    pub fn zh_reason(&self) -> String {
        use ValidationIssue::*;
        match self {
            EmptyEnglish => "缺少英文句子".into(),
            EmptyChinese => "缺少中文翻译".into(),
            NoWords => "缺少逐词标注".into(),
            TokenMismatch { .. } => "逐词标注与句子不一致".into(),
            UnknownPosTag { tag, .. } => format!("未知词性标签「{tag}」"),
            UnknownRoleTag { tag, .. } => format!("未知成分标签「{tag}」"),
            ChunkIndexOutOfRange { .. } => "成分索引越界".into(),
            ChunkOverlap { .. } => "成分划分重叠".into(),
            ChunkGap { .. } => "成分划分未覆盖全句".into(),
            BadIpaChars { ipa, .. } => format!("音标含非法字符「{ipa}」"),
            MissingIpa { .. } => "缺少音标".into(),
            Grammar { problem } => problem.zh_reason(),
            OverLevel { word, .. } => format!("「{word}」超出当前等级的常用词范围"),
            UnknownWord { word } => format!("「{word}」不在常用词表内"),
            TooLong { len, max } => format!("句长 {len} 超出上限 {max}"),
            NearDuplicate { .. } => "与已有句子重复".into(),
            IpaReconciled { .. } => "音标已按词典校正".into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Fatal,
    Repairable,
    Level,
    Duplicate,
    AutoFixed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerdictKind {
    /// Clean (possibly with auto-fixes applied).
    Pass,
    /// Structurally fine, needs a repair call for IPA gaps.
    NeedsRepair,
    /// Structurally fine, wrong level (factory: relevel; user: 可捞回).
    OverLevel,
    /// Near-duplicate of accepted content.
    Duplicate,
    /// Broken data — dropped, never stored.
    Broken,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationReport {
    pub verdict: VerdictKind,
    pub issues: Vec<ValidationIssue>,
    /// Best-effort constructed sentence; present unless the draft was Broken.
    /// (OverLevel/Duplicate sentences back the 丢弃可捞回 UI, §4.4.)
    pub sentence: Option<Sentence>,
    pub simhash: u64,
}

pub struct Validator<'a> {
    pub spec: &'a LevelSpec,
    pub lexicon: &'a Lexicon,
    /// 开放词表模式(场景练习:不分等级,词汇取材于真实生活)。
    /// true 时跳过**全部词表判定**——既不查超带,也不查词表外
    /// (latte/checkout 这类真实词汇本就不在 NGSL 2827 词内);
    /// 结构、成分覆盖、音标、句长、查重照旧强校验。
    /// 见《场景练习模块-实现方案》§3.3。
    pub open_vocabulary: bool,
}

impl<'a> Validator<'a> {
    pub fn new(spec: &'a LevelSpec, lexicon: &'a Lexicon) -> Self {
        Self {
            spec,
            lexicon,
            open_vocabulary: false,
        }
    }

    /// 场景练习用的校验器:词表不设限(其余规则不变)。
    pub fn new_open_vocabulary(spec: &'a LevelSpec, lexicon: &'a Lexicon) -> Self {
        Self {
            spec,
            lexicon,
            open_vocabulary: true,
        }
    }

    pub fn validate(
        &self,
        draft: &DraftSentence,
        scene: &str,
        func: &str,
        dedupe: &DedupeIndex,
    ) -> ValidationReport {
        let mut issues = Vec::new();
        let hash = simhash64(&draft.en);

        // ---- structure ----
        if draft.en.trim().is_empty() {
            issues.push(ValidationIssue::EmptyEnglish);
        }
        if draft.zh.trim().is_empty() {
            issues.push(ValidationIssue::EmptyChinese);
        }
        if draft.words.is_empty() {
            issues.push(ValidationIssue::NoWords);
        }

        let (en_tokens, punct) = tokenize_en(&draft.en);
        let got_tokens: Vec<String> = draft.words.iter().map(|w| w.w.clone()).collect();
        if !draft.words.is_empty()
            && !en_tokens
                .iter()
                .map(|t| t.to_lowercase())
                .eq(got_tokens.iter().map(|t| t.to_lowercase()))
        {
            issues.push(ValidationIssue::TokenMismatch {
                expected: en_tokens.clone(),
                got: got_tokens,
            });
        }

        let mut words: Vec<Word> = Vec::with_capacity(draft.words.len());
        for (i, dw) in draft.words.iter().enumerate() {
            let pos: Option<PosTag> =
                serde_json::from_value(serde_json::Value::String(normalize_pos(&dw.pos))).ok();
            match pos {
                Some(pos) => {
                    let mut ipa = dw.ipa.trim().trim_matches('/').to_string();
                    // Lexicon reconciliation on exact surface match.
                    if let Some(entry) = self.lexicon.exact(&dw.w)
                        && !entry.ipa_gb.is_empty()
                        && entry.ipa_gb != ipa
                    {
                        issues.push(ValidationIssue::IpaReconciled {
                            word_index: i,
                            from: ipa.clone(),
                            to: entry.ipa_gb.clone(),
                        });
                        ipa = entry.ipa_gb.clone();
                    }
                    if ipa.is_empty() {
                        issues.push(ValidationIssue::MissingIpa { word_index: i });
                    } else if !ipa.chars().all(|c| IPA_ALLOWED.contains(c)) {
                        issues.push(ValidationIssue::BadIpaChars {
                            word_index: i,
                            ipa: ipa.clone(),
                        });
                    }
                    words.push(Word {
                        w: dw.w.clone(),
                        ipa,
                        pos,
                    });
                }
                None => {
                    issues.push(ValidationIssue::UnknownPosTag {
                        word_index: i,
                        tag: dw.pos.clone(),
                    });
                }
            }
        }

        // ---- chunks: closed role enum + exact cover, no overlap ----
        let mut chunks: Vec<Chunk> = Vec::with_capacity(draft.chunks.len());
        let mut covered = vec![false; draft.words.len()];
        for (ci, dc) in draft.chunks.iter().enumerate() {
            let role: Option<RoleTag> =
                serde_json::from_value(serde_json::Value::String(normalize_role(&dc.r))).ok();
            let Some(role) = role else {
                issues.push(ValidationIssue::UnknownRoleTag {
                    chunk_index: ci,
                    tag: dc.r.clone(),
                });
                continue;
            };
            for &wi in &dc.i {
                if wi >= draft.words.len() {
                    issues.push(ValidationIssue::ChunkIndexOutOfRange {
                        chunk_index: ci,
                        index: wi,
                    });
                } else if covered[wi] {
                    issues.push(ValidationIssue::ChunkOverlap { word_index: wi });
                } else {
                    covered[wi] = true;
                }
            }
            chunks.push(Chunk {
                r: role,
                i: dc.i.clone(),
            });
        }
        let gaps: Vec<usize> = covered
            .iter()
            .enumerate()
            .filter(|(_, c)| !**c)
            .map(|(i, _)| i)
            .collect();
        if !draft.words.is_empty() && !gaps.is_empty() {
            issues.push(ValidationIssue::ChunkGap { word_indices: gaps });
        }

        // ---- grammar ----
        // 只在结构本身没问题时查:成分没覆盖全、词性非法的句子已经要丢了,
        // 再报一串语法问题只会让丢弃原因变得难读。
        let structurally_sound = !issues.iter().any(|i| i.severity() == Severity::Fatal)
            && words.len() == draft.words.len();
        if structurally_sound {
            let is_base_form = |v: &str| self.lexicon.exact(v).is_some();
            for problem in crate::grammar::check(&draft.en, &words, &chunks, &punct, &is_base_form)
            {
                issues.push(ValidationIssue::Grammar { problem });
            }
        }

        // ---- level ----
        if draft.words.len() > self.spec.max_words as usize {
            issues.push(ValidationIssue::TooLong {
                len: draft.words.len(),
                max: self.spec.max_words,
            });
        }
        for (i, dw) in draft.words.iter().enumerate() {
            if self.open_vocabulary || vocabulary_exempt(&dw.w, &dw.pos, i) {
                continue;
            }
            match self.lexicon.band_of(&dw.w) {
                Some(band) if self.spec.vocab_band > 0 && band > self.spec.vocab_band => {
                    issues.push(ValidationIssue::OverLevel {
                        word: dw.w.clone(),
                        band,
                        allowed: self.spec.vocab_band,
                    });
                }
                Some(_) => {}
                // `vocab_band == 0` = 该等级不设词表带上限(L6 的 spec 注释:
                // "2800+短语动词:不设词表带上限,由抽审把关")。此前词表外
                // 仍旧拦截,等于上限根本没放开 —— 与规格自相矛盾。
                None if self.spec.vocab_band == 0 => {}
                None => issues.push(ValidationIssue::UnknownWord { word: dw.w.clone() }),
            }
        }

        // ---- dedupe ----
        // 精确词形相似度,只跟**同场景**已接受的句子比(见 crate::dedupe)。
        // 旧实现用 simhash 汉明距离 ≤16 判全库近重,阈值落在"不同句"分布
        // 内部,库到 5000 句时会误杀约四成好句。
        if let Some(sim) = dedupe.nearest_similarity(&DedupeKey::of(&draft.en))
            && sim >= NEAR_DUP_MIN_SIMILARITY
        {
            issues.push(ValidationIssue::NearDuplicate { similarity: sim });
        }

        // ---- verdict ----
        let has = |s: Severity| issues.iter().any(|i| i.severity() == s);
        let verdict = if has(Severity::Fatal) {
            VerdictKind::Broken
        } else if has(Severity::Duplicate) {
            VerdictKind::Duplicate
        } else if has(Severity::Level) {
            VerdictKind::OverLevel
        } else if has(Severity::Repairable) {
            VerdictKind::NeedsRepair
        } else {
            VerdictKind::Pass
        };

        let sentence = (verdict != VerdictKind::Broken).then(|| Sentence {
            id: 0,
            level: self.spec.id,
            scene: scene.to_string(),
            func: func.to_string(),
            pattern: draft.pattern.clone(),
            zh: normalize_zh_punctuation(draft.zh.trim()),
            en: draft.en.trim().to_string(),
            punct,
            words,
            chunks,
            note: normalize_zh_punctuation(draft.note.trim()),
            simhash: hash,
        });

        ValidationReport {
            verdict,
            issues,
            sentence,
            simhash: hash,
        }
    }
}

/// 中文标点归一:**紧挨汉字**的半角标点转成全角。
///
/// 实测 685 句 L1 里,中文问号 182 句用半角 `?`、11 句用全角 `？`;逗号
/// 13 : 12。同一个界面上同时出现「你还好吗?」和「门关着吗？」,看着就是没做完。
/// 句号倒是一致(492 句全是 `。`)—— 说明模型自己也没准。
///
/// 只转**紧挨汉字**的,是为了不动中文里合法的半角内容:`10:30`、
/// `A, B, C`、括号里的英文缩写。判据是前后任一侧为 CJK 字符。
fn normalize_zh_punctuation(zh: &str) -> String {
    fn is_cjk(c: char) -> bool {
        matches!(c as u32, 0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0x3000..=0x303F)
    }
    let chars: Vec<char> = zh.chars().collect();
    let mut out = String::with_capacity(zh.len());
    for (i, &c) in chars.iter().enumerate() {
        let full = match c {
            '?' => Some('？'),
            '!' => Some('！'),
            ',' => Some('，'),
            ';' => Some('；'),
            ':' => Some('：'),
            _ => None,
        };
        match full {
            Some(f)
                if chars[..i].last().copied().is_some_and(is_cjk)
                    || chars.get(i + 1).copied().is_some_and(is_cjk)
                    // 句尾标点:前面是汉字就转
                    || (i + 1 == chars.len() && chars[..i].last().copied().is_some_and(is_cjk)) =>
            {
                out.push(f)
            }
            _ => out.push(c),
        }
    }
    out
}

/// 把模型给的词性标签归一到闭集里的写法。
///
/// schema 写死了 14 个标签,但模型常写同义的别名 —— 实测最常见的是给感叹词
/// 标 `int`(闭集里叫 `part`),一次就丢掉一整句。这些别名没有歧义,
/// 认下来不会削弱闭集的意义:标签最终还是那 14 个之一,界面照常渲染。
///
/// 认不出的仍旧报 [`ValidationIssue::UnknownPosTag`] —— 这不是放水。
fn normalize_pos(tag: &str) -> String {
    let t = tag.trim().to_lowercase();
    let canonical = match t.as_str() {
        // 感叹词/否定词/不定式记号 —— 闭集里统归 part
        "int" | "intj" | "interj" | "interjection" | "excl" | "exclamation" | "neg"
        | "negation" | "inf" | "particle" => "part",
        // 限定词
        "det" | "determiner" | "article" => "art",
        // 全称写法
        "noun" => "n",
        "verb" => "v",
        "adjective" => "adj",
        "adverb" => "adv",
        "preposition" => "prep",
        "conjunction" | "conjunc" => "conj",
        "pronoun" => "pron",
        "auxiliary" | "auxverb" | "aux_verb" => "aux",
        "modalverb" | "modal_verb" => "modal",
        "numeral" | "number" | "numeric" => "num",
        "proper" | "propernoun" | "proper_noun" | "propernoun_" => "propn",
        // 疑问词
        "wh_word" | "whword" | "interrogative" => "wh",
        other => other,
    };
    canonical.to_string()
}

/// 成分标签的同义归一(同 [`normalize_pos`] 的理由)。
fn normalize_role(tag: &str) -> String {
    let t = tag.trim().to_lowercase();
    match t.as_str() {
        "subject" => "subj",
        "predicate" | "verb" => "pred",
        "object" => "obj",
        "complement" => "comp",
        "adverbial" | "adv" | "adjunct" => "advl",
        "objectcomplement" | "object_complement" | "oc" => "objc",
        "linking" | "copula" => "link",
        other => other,
    }
    .to_string()
}

/// 免词表判定的 token —— 这些不属于"该等级该不该认识的单词"。
///
/// ① 模型标成 `propn` 的专有名词(原有行为);
/// ② **句中**(非首词)首字母大写的词:英语正字法里几乎只可能是专名。
///    实测模型常把 English / French / Friday 标成 adj/n,凭 pos 一项判不出来,
///    结果整句因"词表外"被丢 —— 大小写是比模型标注更可靠的信号。
///    首词不能用这条(每句首字母都大写),所以只对 `index > 0` 生效;
/// ③ 不含字母的 token:数字、时刻 `10:30`、航班号里的纯数字段。它们不是
///    词汇,却会被 normalize 抹成空串进而判"词表外"。
fn vocabulary_exempt(word: &str, pos: &str, index: usize) -> bool {
    if pos == "propn" {
        return true;
    }
    if !word.chars().any(|c| c.is_ascii_alphabetic()) {
        return true;
    }
    index > 0 && word.chars().next().is_some_and(char::is_uppercase)
}

/// Split `en` into typable tokens + trailing punctuation (句末标点直显不输入).
/// Internal apostrophes/hyphens stay part of the word.
pub fn tokenize_en(en: &str) -> (Vec<String>, String) {
    let trimmed = en.trim();
    let mut punct = String::new();
    let mut body = trimmed;
    while let Some(last) = body.chars().last() {
        if matches!(last, '.' | '!' | '?' | ',' | ';' | ':') {
            punct.insert(0, last);
            body = &body[..body.len() - last.len_utf8()];
        } else {
            break;
        }
    }
    let tokens = body
        .split_whitespace()
        .map(|t| t.trim_matches(|c: char| !(c.is_ascii_alphanumeric() || c == '\'' || c == '-')))
        .filter(|t| !t.is_empty())
        .map(|t| t.to_string())
        .collect();
    (tokens, punct)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::{DraftChunk, DraftWord};

    fn spec() -> LevelSpec {
        LevelSpec::from_yaml(
            r#"
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
"#,
        )
        .unwrap()
    }

    fn lexicon() -> Lexicon {
        Lexicon::from_tsv(
            "i\t2\taɪ\taɪ\t我\n\
             be\t1\tbi\tbi\t是\n\
             fine\t300\tfaɪn\tfaɪn\t好\n\
             passport\t2200\tˈpɑːspɔːt\tˈpæspɔːrt\t护照\n",
        )
        .unwrap()
    }

    fn good_draft() -> DraftSentence {
        DraftSentence {
            en: "I am fine.".into(),
            zh: "我很好。".into(),
            pattern: "主+系+表".into(),
            speaker: String::new(),
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
                    w: "fine".into(),
                    ipa: "faɪn".into(),
                    pos: "adj".into(),
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
            note: "be 动词表状态。".into(),
        }
    }

    fn run(draft: &DraftSentence) -> ValidationReport {
        let spec = spec();
        let lex = lexicon();
        Validator::new(&spec, &lex).validate(draft, "问候", "打招呼", &DedupeIndex::default())
    }

    #[test]
    fn clean_draft_passes() {
        let r = run(&good_draft());
        assert_eq!(r.verdict, VerdictKind::Pass, "issues: {:?}", r.issues);
        let s = r.sentence.unwrap();
        assert_eq!(s.punct, ".");
        assert_eq!(s.en, "I am fine.");
        assert_eq!(s.words.len(), 3);
    }

    #[test]
    fn token_mismatch_is_broken() {
        let mut d = good_draft();
        d.words[2].w = "great".into();
        let r = run(&d);
        assert_eq!(r.verdict, VerdictKind::Broken);
        assert!(r.sentence.is_none());
    }

    #[test]
    fn unknown_pos_is_broken() {
        let mut d = good_draft();
        d.words[0].pos = "banana".into();
        assert_eq!(run(&d).verdict, VerdictKind::Broken);
    }

    #[test]
    fn chunk_gap_is_broken() {
        let mut d = good_draft();
        d.chunks.pop();
        assert_eq!(run(&d).verdict, VerdictKind::Broken);
    }

    #[test]
    fn chunk_overlap_is_broken() {
        let mut d = good_draft();
        d.chunks[1].i = vec![0, 1];
        assert_eq!(run(&d).verdict, VerdictKind::Broken);
    }

    #[test]
    fn over_band_word_flags_over_level() {
        let mut d = good_draft();
        d.en = "I am passport.".into(); // nonsense, but band 2200 > 500
        d.words[2] = DraftWord {
            w: "passport".into(),
            ipa: "ˈpɑːspɔːt".into(),
            pos: "n".into(),
        };
        let r = run(&d);
        assert_eq!(r.verdict, VerdictKind::OverLevel);
        assert!(
            r.sentence.is_some(),
            "over-level sentences must be recoverable"
        );
    }

    #[test]
    fn proper_noun_exempt_from_band() {
        let mut d = good_draft();
        d.en = "I am Tom.".into();
        d.words[2] = DraftWord {
            w: "Tom".into(),
            ipa: "tɒm".into(),
            pos: "propn".into(),
        };
        let r = run(&d);
        assert_eq!(r.verdict, VerdictKind::Pass, "issues: {:?}", r.issues);
    }

    #[test]
    fn ipa_reconciled_from_lexicon() {
        let mut d = good_draft();
        d.words[2].ipa = "fain".into(); // drifted IPA; lexicon says faɪn
        let r = run(&d);
        assert_eq!(r.verdict, VerdictKind::Pass);
        assert_eq!(r.sentence.unwrap().words[2].ipa, "faɪn");
        assert!(
            r.issues
                .iter()
                .any(|i| matches!(i, ValidationIssue::IpaReconciled { .. }))
        );
    }

    #[test]
    fn missing_ipa_needs_repair() {
        let mut d = good_draft();
        d.words[1].ipa = String::new(); // "am" is not in the tiny lexicon
        let r = run(&d);
        assert_eq!(r.verdict, VerdictKind::NeedsRepair);
        assert!(r.sentence.is_some());
    }

    /// 中文标点归一:紧挨汉字的半角转全角,其余不动。
    #[test]
    fn zh_punctuation_normalizes_next_to_cjk() {
        assert_eq!(normalize_zh_punctuation("你还好吗?"), "你还好吗？");
        assert_eq!(normalize_zh_punctuation("太好了!"), "太好了！");
        assert_eq!(
            normalize_zh_punctuation("这是我的朋友,莉莉。"),
            "这是我的朋友，莉莉。"
        );
        // 不挨汉字的半角保持原样:时刻、英文列举
        assert_eq!(normalize_zh_punctuation("10:30 出发"), "10:30 出发");
        assert_eq!(normalize_zh_punctuation("A, B and C"), "A, B and C");
        // 已经是全角的不动
        assert_eq!(normalize_zh_punctuation("你好吗？"), "你好吗？");
    }

    /// 模型常给感叹词标 `int`(闭集里叫 part)、给限定词标 `det` —— 这些
    /// 别名没有歧义,认下来免得一次丢一整句。认不出的仍旧报错,不是放水。
    #[test]
    fn pos_and_role_aliases_normalize() {
        assert_eq!(normalize_pos("int"), "part");
        assert_eq!(normalize_pos("Interjection"), "part");
        assert_eq!(normalize_pos("det"), "art");
        assert_eq!(normalize_pos("noun"), "n");
        assert_eq!(normalize_pos(" VERB "), "v");
        assert_eq!(normalize_pos("n"), "n", "已经是闭集写法就原样");
        assert_eq!(normalize_pos("zzz"), "zzz", "认不出的原样返回,交给闭集报错");
        assert_eq!(normalize_role("subject"), "subj");
        assert_eq!(normalize_role("adverbial"), "advl");
        assert_eq!(normalize_role("advl"), "advl");
    }

    #[test]
    fn near_duplicate_detected() {
        let mut dedupe = DedupeIndex::default();
        dedupe.add("I am fine.");
        let spec = spec();
        let lex = lexicon();
        let r = Validator::new(&spec, &lex).validate(&good_draft(), "s", "f", &dedupe);
        assert_eq!(r.verdict, VerdictKind::Duplicate);
    }

    #[test]
    fn open_vocabulary_accepts_real_world_words_but_keeps_other_rules() {
        // 场景练习:latte 这类真实词汇不在 NGSL 词表内,开放模式必须放行
        let spec = spec();
        let lexicon = lexicon();
        let mut d = good_draft();
        d.en = "I am latte.".into();
        d.words = vec![
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
                w: "latte".into(),
                ipa: "ˈlɑːteɪ".into(),
                pos: "n".into(),
            },
        ];
        d.chunks = vec![
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
        ];

        let strict = Validator::new(&spec, &lexicon).validate(&d, "", "", &DedupeIndex::default());
        assert_eq!(
            strict.verdict,
            VerdictKind::OverLevel,
            "严格模式仍拦词表外词"
        );

        let open = Validator::new_open_vocabulary(&spec, &lexicon).validate(
            &d,
            "",
            "",
            &DedupeIndex::default(),
        );
        assert_eq!(open.verdict, VerdictKind::Pass, "开放词表模式放行");

        // 但结构性错误照旧拦截(成分未覆盖全句)
        let mut broken = d.clone();
        broken.chunks.pop();
        let still = Validator::new_open_vocabulary(&spec, &lexicon).validate(
            &broken,
            "",
            "",
            &DedupeIndex::default(),
        );
        assert_eq!(still.verdict, VerdictKind::Broken, "开放词表不放松结构校验");
    }

    #[test]
    fn too_long_flags_level() {
        let mut d = good_draft();
        let mut words = Vec::new();
        let en: Vec<&str> = vec!["I"; 9];
        for _ in 0..9 {
            words.push(DraftWord {
                w: "I".into(),
                ipa: "aɪ".into(),
                pos: "pron".into(),
            });
        }
        d.en = en.join(" ");
        d.words = words;
        d.chunks = vec![DraftChunk {
            r: "subj".into(),
            i: (0..9).collect(),
        }];
        let r = run(&d);
        assert!(
            r.issues
                .iter()
                .any(|i| matches!(i, ValidationIssue::TooLong { len: 9, max: 8 }))
        );
    }

    #[test]
    fn tokenizer_handles_contractions_and_commas() {
        let (tokens, punct) = tokenize_en("May I see your passport, please?");
        assert_eq!(
            tokens,
            vec!["May", "I", "see", "your", "passport", "please"]
        );
        assert_eq!(punct, "?");
        let (tokens, _) = tokenize_en("Don't worry.");
        assert_eq!(tokens, vec!["Don't", "worry"]);
    }

    /// 分词保留数字,打字板的可上屏字符集必须跟着保留 —— 两边不一致时,
    /// 含数字的词会永远差几格打不满(出厂包「机场值机与安检」实测卡死)。
    /// 前端对应物:`packages/ui/src/practice/TypingBoard.tsx` 的 `CHAR_RE`。
    #[test]
    fn tokenizer_keeps_digits_inside_words() {
        let (tokens, punct) = tokenize_en("Where's the check-in desk for flight BA208?");
        assert_eq!(
            tokens,
            vec![
                "Where's", "the", "check-in", "desk", "for", "flight", "BA208"
            ]
        );
        assert_eq!(punct, "?");
        let (tokens, _) = tokenize_en("I can give you 14A.");
        assert_eq!(tokens, vec!["I", "can", "give", "you", "14A"]);
        // 词内允许的字符集就是这三类:字母、数字、撇号/连字符
        for t in tokens {
            assert!(
                t.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '\'' || c == '-'),
                "意外的词内字符: {t}"
            );
        }
    }
}
