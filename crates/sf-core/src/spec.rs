//! LevelSpec — the single source of truth for "what a level is" (spec §4.9).
//!
//! One YAML file per level lives in `content/specs/`. The same file:
//! * constrains the generation pipeline (vocab band / grammar / length),
//! * drives the validator's reverse checks,
//! * and decides how the practice engine behaves (`practice` section).
//!
//! sf-core only *interprets* the spec; there is no per-level hardcoding
//! anywhere in the engine.

use crate::sentence::LevelId;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SpecError {
    #[error("yaml parse error: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("invalid spec: {0}")]
    Invalid(String),
}

/// How a sentence is practised the first time it is seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlowKind {
    /// 每句先重组后打字(L1–L2 默认).
    ReorderThenTyping,
    /// 直接打字.
    Typing,
    /// 打字/听打混合(复习句按比例转听打).
    Mixed,
}

/// Visibility of a hint element in the practice view.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HintVisibility {
    Always,
    OnClick,
    Hidden,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HintSpec {
    /// 音标显示策略.
    pub ipa: HintVisibility,
    /// 首字母提示.
    #[serde(default)]
    pub first_letter: bool,
    /// 中文题面是否可被用户隐藏(true = 提供"隐藏中文"开关).
    #[serde(default)]
    pub zh_hideable: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JudgeSpec {
    /// 严格模式默认值(错字不上屏);宽松模式为用户设置项.
    pub strict: bool,
    /// 冠词/介词专项统计(L5–L6).
    #[serde(default)]
    pub track_article_preposition: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SrsSpec {
    /// 每日新句默认值.
    pub daily_new_default: u32,
    /// 用户可设范围 [min, max](spec §4.5: 5–50).
    pub daily_new_range: [u32; 2],
    /// 到期复习上限(spec §4.5: 60).
    pub review_cap: u32,
    /// 进入盒 2..=5 时的到期间隔(天).
    pub box_intervals_days: [u32; 4],
    /// 盒 5 抽检间隔(天).
    pub box5_recheck_days: u32,
    /// 听打答对的推进权重(spec §4.5: 1.5).
    pub listening_weight: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PracticeSpec {
    pub flow: FlowKind,
    /// 复习句转听打比例(0.0–1.0;0 = 该级未解锁听打).
    #[serde(default)]
    pub review_listening_ratio: f32,
    /// 盒 ≥ 此值的句子进入默写(0 = 该级未解锁默写).
    #[serde(default)]
    pub dictation_min_box: u8,
    pub hints: HintSpec,
    pub judge: JudgeSpec,
    pub srs: SrsSpec,
}

/// One level's full definition (YAML file shape).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LevelSpec {
    pub id: LevelId,
    /// ≈CEFR 锚点,展示用途, e.g. "A2".
    pub cefr: String,
    /// NGSL 词表带上限(前 n 词;0 = 不限,仅 L6 的短语动词扩展).
    pub vocab_band: u32,
    /// 句长上限(词数).
    pub max_words: u32,
    /// 语法白名单(生成约束 + 校验反查的标签集).
    pub grammar_whitelist: Vec<String>,
    /// 该级典型 can-do 描述(首启等级卡文案).
    pub can_do: Vec<String>,
    pub practice: PracticeSpec,
}

impl LevelSpec {
    /// Parse a single spec from YAML and validate its internal consistency.
    pub fn from_yaml(yaml: &str) -> Result<Self, SpecError> {
        let spec: LevelSpec = serde_yaml::from_str(yaml)?;
        spec.validate()?;
        Ok(spec)
    }

    pub fn validate(&self) -> Result<(), SpecError> {
        let err = |msg: String| Err(SpecError::Invalid(format!("{}: {msg}", self.id)));
        if self.max_words == 0 {
            return err("max_words must be > 0".into());
        }
        let [lo, hi] = self.practice.srs.daily_new_range;
        if lo > hi {
            return err(format!("daily_new_range min {lo} > max {hi}"));
        }
        let d = self.practice.srs.daily_new_default;
        if d < lo || d > hi {
            return err(format!("daily_new_default {d} outside range [{lo}, {hi}]"));
        }
        if !(0.0..=1.0).contains(&self.practice.review_listening_ratio) {
            return err("review_listening_ratio must be within [0, 1]".into());
        }
        if self.practice.dictation_min_box > 5 {
            return err("dictation_min_box must be within 0..=5".into());
        }
        if self.practice.srs.listening_weight < 1.0 {
            return err("listening_weight must be >= 1.0".into());
        }
        if self.practice.srs.review_cap == 0 {
            return err("review_cap must be > 0".into());
        }
        Ok(())
    }

    /// Days until due when a sentence *enters* `box_idx` (1–5).
    /// Box 1 is always due the same moment (spec §4.5: 盒 1 当天).
    pub fn interval_days(&self, box_idx: u8) -> u32 {
        match box_idx {
            0 | 1 => 0,
            2..=5 => self.practice.srs.box_intervals_days[box_idx as usize - 2],
            _ => self.practice.srs.box5_recheck_days,
        }
    }

    /// Interval for a sentence that stays in box 5 after a correct recheck.
    pub fn recheck_days(&self) -> u32 {
        self.practice.srs.box5_recheck_days
    }

    /// Effective daily-new count: user override clamped into the spec range.
    pub fn effective_daily_new(&self, user_override: Option<u32>) -> u32 {
        let [lo, hi] = self.practice.srs.daily_new_range;
        match user_override {
            Some(v) => v.clamp(lo, hi),
            None => self.practice.srs.daily_new_default,
        }
    }
}

/// Parse a multi-document YAML string (one spec per document) — the shape of
/// the `level_spec` snapshot stored in content.db.
pub fn parse_specs(yaml: &str) -> Result<Vec<LevelSpec>, SpecError> {
    let mut specs = Vec::new();
    for doc in serde_yaml::Deserializer::from_str(yaml) {
        let spec = LevelSpec::deserialize(doc)?;
        spec.validate()?;
        specs.push(spec);
    }
    Ok(specs)
}

/// Shared fixture for unit tests across sf-core modules.
#[cfg(test)]
pub(crate) mod tests_support {
    pub(crate) const L3_YAML: &str = r#"
id: L3
cefr: "A2"
vocab_band: 1500
max_words: 12
grammar_whitelist: [past_simple, comparative, adverbs_frequency]
can_do: ["点餐", "约时间"]
practice:
  flow: typing
  review_listening_ratio: 0.3
  dictation_min_box: 0
  hints: { ipa: on_click, first_letter: false, zh_hideable: false }
  judge: { strict: true }
  srs:
    daily_new_default: 20
    daily_new_range: [5, 50]
    review_cap: 60
    box_intervals_days: [1, 3, 7, 14]
    box5_recheck_days: 30
    listening_weight: 1.5
"#;
}

#[cfg(test)]
mod tests {
    use super::*;

    const L3_YAML: &str = r#"
id: L3
cefr: "A2"
vocab_band: 1500
max_words: 12
grammar_whitelist: [past_simple, comparative, adverbs_frequency]
can_do: ["点餐", "约时间"]
practice:
  flow: typing
  review_listening_ratio: 0.3
  dictation_min_box: 0
  hints: { ipa: on_click, first_letter: false, zh_hideable: false }
  judge: { strict: true }
  srs:
    daily_new_default: 20
    daily_new_range: [5, 50]
    review_cap: 60
    box_intervals_days: [1, 3, 7, 14]
    box5_recheck_days: 30
    listening_weight: 1.5
"#;

    #[test]
    fn parses_valid_spec() {
        let spec = LevelSpec::from_yaml(L3_YAML).unwrap();
        assert_eq!(spec.id, LevelId::L3);
        assert_eq!(spec.interval_days(1), 0);
        assert_eq!(spec.interval_days(2), 1);
        assert_eq!(spec.interval_days(5), 14);
        assert_eq!(spec.recheck_days(), 30);
    }

    #[test]
    fn clamps_daily_new_override() {
        let spec = LevelSpec::from_yaml(L3_YAML).unwrap();
        assert_eq!(spec.effective_daily_new(None), 20);
        assert_eq!(spec.effective_daily_new(Some(3)), 5);
        assert_eq!(spec.effective_daily_new(Some(500)), 50);
        assert_eq!(spec.effective_daily_new(Some(30)), 30);
    }

    #[test]
    fn rejects_inconsistent_spec() {
        let bad = L3_YAML.replace("daily_new_default: 20", "daily_new_default: 99");
        assert!(LevelSpec::from_yaml(&bad).is_err());
    }

    #[test]
    fn parses_multi_document() {
        let both = format!("{L3_YAML}\n---\n{}", L3_YAML.replace("id: L3", "id: L4"));
        let specs = parse_specs(&both).unwrap();
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[1].id, LevelId::L4);
    }
}
