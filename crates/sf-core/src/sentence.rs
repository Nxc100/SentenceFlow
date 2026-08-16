//! Sentence data model shared by content.db, user_content.db, the generation
//! pipeline and the practice UI (spec §7.7).
//!
//! POS / ROLE tags are closed enums: the pastel grammar palette (spec §5.2) is
//! keyed by these tags, and the pipeline validator rejects anything outside
//! them, so an unknown tag can never reach the UI.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// L1–L6 (spec §4.9). Ordered so `LevelId::L1 < LevelId::L3` works for
/// band/over-level comparisons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum LevelId {
    L1,
    L2,
    L3,
    L4,
    L5,
    L6,
}

impl LevelId {
    pub const ALL: [LevelId; 6] = [
        LevelId::L1,
        LevelId::L2,
        LevelId::L3,
        LevelId::L4,
        LevelId::L5,
        LevelId::L6,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            LevelId::L1 => "L1",
            LevelId::L2 => "L2",
            LevelId::L3 => "L3",
            LevelId::L4 => "L4",
            LevelId::L5 => "L5",
            LevelId::L6 => "L6",
        }
    }
}

impl fmt::Display for LevelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for LevelId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "L1" => Ok(LevelId::L1),
            "L2" => Ok(LevelId::L2),
            "L3" => Ok(LevelId::L3),
            "L4" => Ok(LevelId::L4),
            "L5" => Ok(LevelId::L5),
            "L6" => Ok(LevelId::L6),
            other => Err(format!("unknown level id: {other}")),
        }
    }
}

/// Part-of-speech tags — exactly the 14 rows of the POS capsule palette
/// (spec §5.2). Serialized as short stable codes (DB/JSON contract).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum PosTag {
    /// 代词
    #[serde(rename = "pron")]
    Pronoun,
    /// 名词
    #[serde(rename = "n")]
    Noun,
    /// 动词
    #[serde(rename = "v")]
    Verb,
    /// be/助动词
    #[serde(rename = "aux")]
    Auxiliary,
    /// 情态动词
    #[serde(rename = "modal")]
    Modal,
    /// 形容词
    #[serde(rename = "adj")]
    Adjective,
    /// 疑问词
    #[serde(rename = "wh")]
    Interrogative,
    /// 副词
    #[serde(rename = "adv")]
    Adverb,
    /// 介词
    #[serde(rename = "prep")]
    Preposition,
    /// 冠词
    #[serde(rename = "art")]
    Article,
    /// 连词
    #[serde(rename = "conj")]
    Conjunction,
    /// 数词
    #[serde(rename = "num")]
    Numeral,
    /// 专有名词
    #[serde(rename = "propn")]
    ProperNoun,
    /// 不定式标记 to / 引导词
    #[serde(rename = "part")]
    Particle,
}

impl PosTag {
    pub const ALL: [PosTag; 14] = [
        PosTag::Pronoun,
        PosTag::Noun,
        PosTag::Verb,
        PosTag::Auxiliary,
        PosTag::Modal,
        PosTag::Adjective,
        PosTag::Interrogative,
        PosTag::Adverb,
        PosTag::Preposition,
        PosTag::Article,
        PosTag::Conjunction,
        PosTag::Numeral,
        PosTag::ProperNoun,
        PosTag::Particle,
    ];

    /// Chinese display name used by the POS capsule.
    pub fn zh_name(&self) -> &'static str {
        match self {
            PosTag::Pronoun => "代词",
            PosTag::Noun => "名词",
            PosTag::Verb => "动词",
            PosTag::Auxiliary => "助动词",
            PosTag::Modal => "情态动词",
            PosTag::Adjective => "形容词",
            PosTag::Interrogative => "疑问词",
            PosTag::Adverb => "副词",
            PosTag::Preposition => "介词",
            PosTag::Article => "冠词",
            PosTag::Conjunction => "连词",
            PosTag::Numeral => "数词",
            PosTag::ProperNoun => "专有名词",
            PosTag::Particle => "引导词",
        }
    }
}

/// Sentence-role tags — exactly the 8 rows of the ROLE card palette
/// (spec §5.2). Serialized as short stable codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RoleTag {
    /// 主语
    #[serde(rename = "subj")]
    Subject,
    /// 谓语
    #[serde(rename = "pred")]
    Predicate,
    /// 系动词
    #[serde(rename = "link")]
    Linking,
    /// 宾语
    #[serde(rename = "obj")]
    Object,
    /// 表语
    #[serde(rename = "comp")]
    Complement,
    /// 状语(时间/地点/方式)
    #[serde(rename = "advl")]
    Adverbial,
    /// 宾语补足语
    #[serde(rename = "objc")]
    ObjectComplement,
    /// 引导词/固定表达
    #[serde(rename = "marker")]
    Marker,
}

impl RoleTag {
    pub const ALL: [RoleTag; 8] = [
        RoleTag::Subject,
        RoleTag::Predicate,
        RoleTag::Linking,
        RoleTag::Object,
        RoleTag::Complement,
        RoleTag::Adverbial,
        RoleTag::ObjectComplement,
        RoleTag::Marker,
    ];

    pub fn zh_name(&self) -> &'static str {
        match self {
            RoleTag::Subject => "主语",
            RoleTag::Predicate => "谓语",
            RoleTag::Linking => "系动词",
            RoleTag::Object => "宾语",
            RoleTag::Complement => "表语",
            RoleTag::Adverbial => "状语",
            RoleTag::ObjectComplement => "宾补",
            RoleTag::Marker => "引导词",
        }
    }
}

/// One word of a sentence with its teaching annotations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Word {
    /// Surface form as typed (no punctuation).
    pub w: String,
    /// British IPA, no surrounding slashes, e.g. `ˈrestrɒnt`.
    pub ipa: String,
    pub pos: PosTag,
}

/// One sentence-role chunk: role + indices into `words` (0-based, ascending,
/// contiguous runs preferred but not required by the model).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Chunk {
    pub r: RoleTag,
    pub i: Vec<usize>,
}

/// Fully-annotated sentence — the row shape of `sentence` in content.db /
/// user_content.db (spec §7.7).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sentence {
    pub id: i64,
    pub level: LevelId,
    /// Scene tag, e.g. "餐厅" — groups sentences in the library.
    pub scene: String,
    /// Communicative function, e.g. "点餐".
    #[serde(default)]
    pub func: String,
    /// Sentence pattern formula, e.g. "主 + 谓 + 宾".
    #[serde(default)]
    pub pattern: String,
    pub zh: String,
    pub en: String,
    /// Trailing punctuation displayed but never typed (spec §4.1).
    #[serde(default)]
    pub punct: String,
    pub words: Vec<Word>,
    pub chunks: Vec<Chunk>,
    /// One-sentence teaching note shown in the parse view.
    #[serde(default)]
    pub note: String,
    /// 64-bit simhash over normalized words, for dedupe (spec §7.4).
    #[serde(default)]
    pub simhash: u64,
}

impl Sentence {
    /// Typable target words (lowercased forms are the judge's targets).
    pub fn target_words(&self) -> Vec<&str> {
        self.words.iter().map(|w| w.w.as_str()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pos_codes_roundtrip() {
        for pos in PosTag::ALL {
            let s = serde_json::to_string(&pos).unwrap();
            let back: PosTag = serde_json::from_str(&s).unwrap();
            assert_eq!(pos, back);
        }
    }

    #[test]
    fn role_codes_roundtrip() {
        for role in RoleTag::ALL {
            let s = serde_json::to_string(&role).unwrap();
            let back: RoleTag = serde_json::from_str(&s).unwrap();
            assert_eq!(role, back);
        }
    }

    #[test]
    fn level_orders() {
        assert!(LevelId::L1 < LevelId::L6);
        assert_eq!("L4".parse::<LevelId>().unwrap(), LevelId::L4);
        assert!("L7".parse::<LevelId>().is_err());
    }

    #[test]
    fn sentence_json_shape() {
        let s = Sentence {
            id: 1,
            level: LevelId::L1,
            scene: "问候".into(),
            func: "打招呼".into(),
            pattern: "主+系+表".into(),
            zh: "我很好。".into(),
            en: "I am fine".into(),
            punct: ".".into(),
            words: vec![
                Word {
                    w: "I".into(),
                    ipa: "aɪ".into(),
                    pos: PosTag::Pronoun,
                },
                Word {
                    w: "am".into(),
                    ipa: "æm".into(),
                    pos: PosTag::Auxiliary,
                },
                Word {
                    w: "fine".into(),
                    ipa: "faɪn".into(),
                    pos: PosTag::Adjective,
                },
            ],
            chunks: vec![
                Chunk {
                    r: RoleTag::Subject,
                    i: vec![0],
                },
                Chunk {
                    r: RoleTag::Linking,
                    i: vec![1],
                },
                Chunk {
                    r: RoleTag::Complement,
                    i: vec![2],
                },
            ],
            note: "be 动词表状态".into(),
            simhash: 0,
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(json.contains("\"pos\":\"pron\""));
        assert!(json.contains("\"r\":\"subj\""));
        let back: Sentence = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }
}
