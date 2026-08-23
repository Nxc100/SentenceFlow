//! 确定性语法检查(spec §7.4 校验器的一部分)。
//!
//! # 边界:只查"能证明错"的
//!
//! 语法正确性大体上判不了 —— 判了就会开始误杀好句子,而误杀正是这条流水线
//! 最大的成本来源。所以这里只收**能用手上已有的标注证明是错的**几类,
//! 每一类都要求近乎零假阳性:
//!
//! | 检查 | 靠什么判 | 为什么可靠 |
//! |------|----------|-----------|
//! | a / an | **下一个词的 IPA** | 冠词看的是读音不是拼写;IPA 正好给了读音,`an hour`(ˈaʊə,元音)与 `a university`(ˌjuːn…,辅音)都能判对 |
//! | 三单一致 | 主语成分 + 谓语成分 | 只查 he/she/it 这类明确单数代词主语 + 动词原形 |
//! | 相邻重复词 | 词序列 | `the the` 只可能是错 |
//! | 首字母大写 | 首词 | 英语句子必大写 |
//! | wh 疑问句标点 | 首词 pos + 句末标点 | wh 开头必以 ? 或 ! 收尾 |
//!
//! 判不了因而**不查**的:时态一致、冠词该不该有、介词搭配、语序自然度、
//! 是否符合该等级的 `grammar_whitelist`。这些交给 5% 人工抽审(§8)。
//!
//! # 严重度
//!
//! 全部按**可修补**(Repairable)处理,不是直接丢弃:这几类都是模型一次
//! 修补就能改对的小错,而句子本身的内容是好的。修不好才丢。

use serde::{Deserialize, Serialize};
use sf_core::{Chunk, RoleTag, Word};

/// 一条语法问题。`index` 是词序号(0 起)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GrammarProblem {
    /// `a apple` / `an book` —— 冠词与后词读音不匹配。
    ArticleSound {
        index: usize,
        article: String,
        next: String,
        next_is_vowel: bool,
    },
    /// `he go` —— 单数代词主语配了动词原形。
    ThirdPersonSingular { subject: String, verb: String },
    /// `the the` —— 相邻重复词。
    RepeatedWord { index: usize, word: String },
    /// 句首未大写。
    LowercaseStart { word: String },
    /// wh 开头却不是问句标点。
    WhQuestionPunct { first: String, punct: String },
}

impl GrammarProblem {
    pub fn zh_reason(&self) -> String {
        match self {
            Self::ArticleSound {
                article,
                next,
                next_is_vowel,
                ..
            } => {
                let want = if *next_is_vowel { "an" } else { "a" };
                format!("「{article} {next}」冠词用错,应为「{want} {next}」")
            }
            Self::ThirdPersonSingular { subject, verb } => {
                format!("主谓不一致:「{subject}」后的动词「{verb}」应用第三人称单数形式")
            }
            Self::RepeatedWord { word, .. } => format!("重复的词「{word}」"),
            Self::LowercaseStart { word } => format!("句首「{word}」未大写"),
            Self::WhQuestionPunct { first, .. } => {
                format!("「{first}」开头的疑问句应以问号结尾")
            }
        }
    }
}

/// IPA 里的元音音素起始。冠词看读音:`an hour`(ˈaʊə)、`a university`(ˌjuːn…)。
const IPA_VOWELS: &str = "æɑɒɔəɜɛɪʊʌaeiou";

/// 这个 IPA 是不是以元音音素开头(跳过重音符号)。
/// IPA 为空时返回 None —— 判不了就不判,别猜。
fn starts_with_vowel(ipa: &str) -> Option<bool> {
    let first = ipa.chars().find(|c| !matches!(c, 'ˈ' | 'ˌ' | ' ' | '.'))?;
    Some(IPA_VOWELS.contains(first))
}

/// 形态与原形同形、看不出时态的动词。`He read the book.` 既可能是过去式
/// (正确)也可能是现在时漏了 -s(错误),分不清就不判。
const INVARIANT_VERBS: &[&str] = &[
    "read", "put", "cut", "let", "set", "hit", "cost", "hurt", "shut", "spread", "quit", "bet",
    "burst", "cast",
];

/// 合法的相邻重复:`had had`(过去完成)、`that that`(从句套从句)。
const REPEAT_OK: &[&str] = &["had", "that"];

/// 明确的第三人称单数代词主语。名词主语判不出单复数(pos 不带数),不查。
const THIRD_SINGULAR: &[&str] = &["he", "she", "it"];

/// 查一句的语法。`words`/`chunks` 是已过结构校验的标注,`punct` 是句末标点。
///
/// `is_base_form` 由调用方提供:给定动词表面形,回答"它是不是词表里的原形"
/// —— 这需要词表,而本模块保持无依赖(纯函数,好测)。
pub fn check(
    en: &str,
    words: &[Word],
    chunks: &[Chunk],
    punct: &str,
    is_base_form: &dyn Fn(&str) -> bool,
) -> Vec<GrammarProblem> {
    let mut out = Vec::new();
    if words.is_empty() {
        return out;
    }

    // ---- 句首大写 ----
    // 看 `en` 原文而不是 words[0].w:模型常把 `"en":"See you tomorrow."` 的
    // words[0].w 写成小写 `see`(逐词校验大小写不敏感,过得去)。句子本身
    // 没错,拿词标注判会误报 —— 实测 20 句里误报 3 句。
    let first = &words[0].w;
    if en
        .trim()
        .chars()
        .find(|c| c.is_alphabetic())
        .is_some_and(|c| c.is_lowercase())
    {
        out.push(GrammarProblem::LowercaseStart {
            word: en.split_whitespace().next().unwrap_or(first).to_string(),
        });
    }

    // ---- wh 疑问句标点 ----
    if words[0].pos == sf_core::PosTag::Interrogative
        && !punct.contains('?')
        && !punct.contains('!')
    {
        out.push(GrammarProblem::WhQuestionPunct {
            first: first.clone(),
            punct: punct.to_string(),
        });
    }

    // ---- 相邻重复词 ----
    for (i, pair) in words.windows(2).enumerate() {
        let (a, b) = (pair[0].w.to_lowercase(), pair[1].w.to_lowercase());
        if a == b && !REPEAT_OK.contains(&a.as_str()) {
            out.push(GrammarProblem::RepeatedWord {
                index: i + 1,
                word: b,
            });
        }
    }

    // ---- a / an ----
    for (i, w) in words.iter().enumerate() {
        let article = w.w.to_lowercase();
        if article != "a" && article != "an" {
            continue;
        }
        let Some(next) = words.get(i + 1) else {
            continue;
        };
        let Some(vowel) = starts_with_vowel(&next.ipa) else {
            continue; // 没音标就判不了
        };
        let wrong = (vowel && article == "a") || (!vowel && article == "an");
        if wrong {
            out.push(GrammarProblem::ArticleSound {
                index: i,
                article: w.w.clone(),
                next: next.w.clone(),
                next_is_vowel: vowel,
            });
        }
    }

    // ---- 三单一致 ----
    if let Some(problem) = third_person_check(words, chunks, is_base_form) {
        out.push(problem);
    }

    out
}

fn third_person_check(
    words: &[Word],
    chunks: &[Chunk],
    is_base_form: &dyn Fn(&str) -> bool,
) -> Option<GrammarProblem> {
    // 主语必须是单独一个明确的三单代词
    let subj = chunks.iter().find(|c| c.r == RoleTag::Subject)?;
    let [si] = subj.i[..] else { return None };
    let subject = words.get(si)?.w.to_lowercase();
    if !THIRD_SINGULAR.contains(&subject.as_str()) {
        return None;
    }
    // 谓语的第一个词必须是实义动词(情态/助动词自带形态,不查)
    let pred = chunks.iter().find(|c| c.r == RoleTag::Predicate)?;
    let vi = *pred.i.first()?;
    let verb_word = words.get(vi)?;
    if verb_word.pos != sf_core::PosTag::Verb {
        return None;
    }
    let verb = verb_word.w.to_lowercase();
    if INVARIANT_VERBS.contains(&verb.as_str()) {
        return None;
    }
    // 原形 = 词表里就是这个形态,且不以 s 结尾(goes/likes 查不到原形)
    if verb.ends_with('s') || !is_base_form(&verb) {
        return None;
    }
    Some(GrammarProblem::ThirdPersonSingular {
        subject: words.get(si)?.w.clone(),
        verb: verb_word.w.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sf_core::PosTag;

    fn w(word: &str, ipa: &str, pos: PosTag) -> Word {
        Word {
            w: word.into(),
            ipa: ipa.into(),
            pos,
        }
    }
    fn chunk(r: RoleTag, i: &[usize]) -> Chunk {
        Chunk { r, i: i.to_vec() }
    }
    /// 测试用的"原形表":这几个词是原形。
    fn base(v: &str) -> bool {
        ["go", "like", "want", "read", "eat"].contains(&v)
    }

    #[test]
    fn clean_sentence_has_no_problems() {
        let words = vec![
            w("I", "aɪ", PosTag::Pronoun),
            w("want", "wɒnt", PosTag::Verb),
            w("an", "ən", PosTag::Article),
            w("apple", "ˈæpl", PosTag::Noun),
        ];
        let chunks = vec![
            chunk(RoleTag::Subject, &[0]),
            chunk(RoleTag::Predicate, &[1]),
            chunk(RoleTag::Object, &[2, 3]),
        ];
        assert!(check("I want an apple.", &words, &chunks, ".", &base).is_empty());
    }

    #[test]
    fn article_follows_pronunciation_not_spelling() {
        // a + 元音音素 = 错
        let bad = vec![
            w("I", "aɪ", PosTag::Pronoun),
            w("a", "ə", PosTag::Article),
            w("apple", "ˈæpl", PosTag::Noun),
        ];
        let p = check("I a apple.", &bad, &[], ".", &base);
        assert!(matches!(p[0], GrammarProblem::ArticleSound { .. }), "{p:?}");
        assert!(p[0].zh_reason().contains("an apple"));

        // an hour:h 不发音,IPA 以元音起 —— 必须放行
        // (放进完整句里测,免得触发"句首未大写")
        let hour = vec![
            w("It", "ɪt", PosTag::Pronoun),
            w("takes", "teɪks", PosTag::Verb),
            w("an", "ən", PosTag::Article),
            w("hour", "ˈaʊə", PosTag::Noun),
        ];
        assert!(
            check("It takes an hour.", &hour, &[], ".", &base).is_empty(),
            "an hour 是对的"
        );

        // a university:u 是元音字母但读 /j/ —— 必须放行
        let uni = vec![
            w("It", "ɪt", PosTag::Pronoun),
            w("is", "ɪz", PosTag::Auxiliary),
            w("a", "ə", PosTag::Article),
            w("university", "ˌjuːnɪˈvɜːsəti", PosTag::Noun),
        ];
        assert!(
            check("It is a university.", &uni, &[], ".", &base).is_empty(),
            "a university 是对的"
        );

        // an + 辅音音素 = 错
        let an_book = vec![
            w("I", "aɪ", PosTag::Pronoun),
            w("an", "ən", PosTag::Article),
            w("book", "bʊk", PosTag::Noun),
        ];
        let p = check("I an book.", &an_book, &[], ".", &base);
        assert!(p[0].zh_reason().contains("a book"), "{p:?}");

        // 没音标就不判,别猜
        let unknown = vec![
            w("I", "aɪ", PosTag::Pronoun),
            w("a", "ə", PosTag::Article),
            w("xyz", "", PosTag::Noun),
        ];
        assert!(check("I a xyz.", &unknown, &[], ".", &base).is_empty());
    }

    #[test]
    fn third_person_singular_needs_s() {
        let words = vec![
            w("He", "hiː", PosTag::Pronoun),
            w("go", "ɡəʊ", PosTag::Verb),
            w("home", "həʊm", PosTag::Noun),
        ];
        let chunks = vec![
            chunk(RoleTag::Subject, &[0]),
            chunk(RoleTag::Predicate, &[1]),
            chunk(RoleTag::Object, &[2]),
        ];
        let p = check("I want an apple.", &words, &chunks, ".", &base);
        assert!(
            matches!(p[0], GrammarProblem::ThirdPersonSingular { .. }),
            "{p:?}"
        );
    }

    #[test]
    fn third_person_check_stays_quiet_when_it_cannot_be_sure() {
        let mk = |verb: &str, ipa: &str, pos: PosTag| {
            (
                vec![
                    w("He", "hiː", PosTag::Pronoun),
                    w(verb, ipa, pos),
                    w("it", "ɪt", PosTag::Pronoun),
                ],
                vec![
                    chunk(RoleTag::Subject, &[0]),
                    chunk(RoleTag::Predicate, &[1]),
                ],
            )
        };
        // goes:不是原形
        let (ws, cs) = mk("goes", "ɡəʊz", PosTag::Verb);
        assert!(check("He goes it.", &ws, &cs, ".", &base).is_empty());
        // read:过去式与原形同形,分不清就不判
        let (ws, cs) = mk("read", "red", PosTag::Verb);
        assert!(check("He goes it.", &ws, &cs, ".", &base).is_empty());
        // 情态/助动词自带形态
        let (ws, cs) = mk("can", "kæn", PosTag::Modal);
        assert!(check("He goes it.", &ws, &cs, ".", &base).is_empty());
        // 复数/名词主语判不出数,不查
        let ws = vec![
            w("They", "ðeɪ", PosTag::Pronoun),
            w("go", "ɡəʊ", PosTag::Verb),
        ];
        let cs = vec![
            chunk(RoleTag::Subject, &[0]),
            chunk(RoleTag::Predicate, &[1]),
        ];
        assert!(check("He goes it.", &ws, &cs, ".", &base).is_empty());
    }

    #[test]
    fn repeated_words_flagged_except_legit_pairs() {
        let dup = vec![
            w("The", "ðə", PosTag::Article),
            w("the", "ðə", PosTag::Article),
            w("cat", "kæt", PosTag::Noun),
        ];
        assert!(matches!(
            check("The the cat.", &dup, &[], ".", &base)[0],
            GrammarProblem::RepeatedWord { .. }
        ));
        // had had 合法
        let ok = vec![
            w("I", "aɪ", PosTag::Pronoun),
            w("had", "hæd", PosTag::Auxiliary),
            w("had", "hæd", PosTag::Verb),
        ];
        assert!(check("I had had it.", &ok, &[], ".", &base).is_empty());
    }

    #[test]
    fn first_letter_and_wh_punctuation() {
        let lower = vec![
            w("how", "haʊ", PosTag::Interrogative),
            w("much", "mʌtʃ", PosTag::Adverb),
        ];
        let p = check("how much.", &lower, &[], ".", &base);
        assert_eq!(p.len(), 2, "小写开头 + wh 缺问号,应各报一条:{p:?}");

        let ok = vec![
            w("How", "haʊ", PosTag::Interrogative),
            w("much", "mʌtʃ", PosTag::Adverb),
        ];
        assert!(check("How much?", &ok, &[], "?", &base).is_empty());
        // 感叹句也放行:What a nice day!
        assert!(check("How much!", &ok, &[], "!", &base).is_empty());
    }
}
