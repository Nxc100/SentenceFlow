//! Seed sentence files — the hand-authored factory content source
//! (`content/seed/*.yaml`, format in `content/seed/README.md`).
//!
//! Seeds flow through the *same* validation pipeline as generated content:
//! [`crate::validate::Validator`] decides, seeds get no special treatment.

use crate::parse::{DraftChunk, DraftSentence, DraftWord};
use serde::{Deserialize, Serialize};
use sf_core::sentence::LevelId;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedFile {
    pub level: LevelId,
    pub sentences: Vec<SeedSentence>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedSentence {
    pub en: String,
    pub zh: String,
    #[serde(default)]
    pub scene: String,
    #[serde(default)]
    pub func: String,
    #[serde(default)]
    pub pattern: String,
    #[serde(default)]
    pub note: String,
    pub words: Vec<DraftWord>,
    pub chunks: Vec<DraftChunk>,
    /// 场景对话包专用:说话人 A/B(普通种子句留空)。
    #[serde(default)]
    pub speaker: String,
}

impl SeedFile {
    pub fn from_yaml(yaml: &str) -> Result<Self, String> {
        serde_yaml::from_str(yaml).map_err(|e| e.to_string())
    }
}

impl SeedSentence {
    /// Convert to the pipeline's pre-validation shape.
    pub fn to_draft(&self) -> DraftSentence {
        DraftSentence {
            en: self.en.clone(),
            zh: self.zh.clone(),
            pattern: self.pattern.clone(),
            words: self.words.clone(),
            chunks: self.chunks.clone(),
            note: self.note.clone(),
            speaker: self.speaker.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_seed_yaml() {
        let yaml = r#"
level: L1
sentences:
  - en: "I am fine."
    zh: "我很好。"
    scene: "问候"
    func: "回应问候"
    pattern: "主+系+表"
    note: "be 动词表状态。"
    words:
      - { w: "I", ipa: "aɪ", pos: "pron" }
      - { w: "am", ipa: "æm", pos: "aux" }
      - { w: "fine", ipa: "faɪn", pos: "adj" }
    chunks:
      - { r: "subj", i: [0] }
      - { r: "link", i: [1] }
      - { r: "comp", i: [2] }
"#;
        let seed = SeedFile::from_yaml(yaml).unwrap();
        assert_eq!(seed.level, LevelId::L1);
        assert_eq!(seed.sentences.len(), 1);
        let draft = seed.sentences[0].to_draft();
        assert_eq!(draft.words.len(), 3);
        assert_eq!(draft.words[0].pos, "pron");
    }
}
