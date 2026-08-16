//! User settings (spec §4.8) — one JSON document in progress.db `kv`.
//!
//! Every field has a serde default matching the spec's default column, so a
//! settings blob from any older version deserializes cleanly (additive
//! migration by construction).

use serde::{Deserialize, Serialize};
use sf_core::sentence::LevelId;
use sf_llm::types::ChannelId;

pub const SETTINGS_KEY: &str = "settings";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Settings {
    pub practice: PracticeSettings,
    pub sound: SoundSettings,
    pub appearance: AppearanceSettings,
    pub accessibility: AccessibilitySettings,
    pub ai: AiSettings,
    /// 当前练习等级(首启定级结果).
    pub level: Option<LevelId>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PracticeSettings {
    /// 严格打字(错字不上屏).
    pub strict_typing: bool,
    /// 自动朗读答案.
    pub auto_speak_answer: bool,
    /// 隐藏中文(仅在该级 spec 允许时生效).
    pub hide_chinese: bool,
    /// 每日新句数覆盖(None = 随等级).
    pub daily_new: Option<u32>,
    /// 先重组后打字覆盖(None = 随等级).
    pub reorder_first: Option<bool>,
}

impl Default for PracticeSettings {
    fn default() -> Self {
        Self {
            strict_typing: true,
            auto_speak_answer: true,
            hide_chinese: false,
            daily_new: None,
            reorder_first: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Accent {
    Gb,
    Us,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeySound {
    Off,
    Soft,
    Mechanical,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SoundSettings {
    pub accent: Accent,
    /// 语速 0.6–1.4×.
    pub rate: f32,
    pub key_sound: KeySound,
    /// 效果音量 0–100.
    pub fx_volume: u8,
}

impl Default for SoundSettings {
    fn default() -> Self {
        Self {
            accent: Accent::Gb,
            rate: 1.0,
            key_sound: KeySound::Soft,
            fx_volume: 70,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    Light,
    Dark,
    System,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FontSize {
    Small,
    Medium,
    Large,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppearanceSettings {
    pub theme: Theme,
    /// 护眼纸色.
    pub paper: bool,
    pub practice_font_size: FontSize,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            paper: false,
            practice_font_size: FontSize::Medium,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriState {
    System,
    On,
    Off,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AccessibilitySettings {
    pub reduce_motion: TriState,
    pub dyslexic_font: bool,
    pub high_contrast: bool,
    pub color_blind_friendly: bool,
}

impl Default for AccessibilitySettings {
    fn default() -> Self {
        Self {
            reduce_motion: TriState::System,
            dyslexic_font: false,
            high_contrast: false,
            color_blind_friendly: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiSettings {
    /// 所选通道(None = 未配置,生成工坊为引导态).
    pub channel: Option<ChannelId>,
    /// 所选模型 id.
    pub model: Option<String>,
    /// 单次预算上限,默认 ¥1 (§4.7).
    pub per_run_budget_cny: f64,
    /// 月度提醒阈值(None = 关).
    pub monthly_reminder_cny: Option<f64>,
    /// 用户手改价格表(None = 用 channels.json).
    pub price_override: Option<sf_llm::meter::PriceTable>,
    /// opencode 手动指定路径 (§6.4).
    pub opencode_bin: Option<String>,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            channel: None,
            model: None,
            per_run_budget_cny: 1.0,
            monthly_reminder_cny: None,
            price_override: None,
            opencode_bin: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_spec_table() {
        let s = Settings::default();
        assert!(s.practice.strict_typing);
        assert!(s.practice.auto_speak_answer);
        assert!(!s.practice.hide_chinese);
        assert_eq!(s.sound.accent, Accent::Gb);
        assert_eq!(s.sound.fx_volume, 70);
        assert_eq!(s.appearance.theme, Theme::System);
        assert_eq!(s.accessibility.reduce_motion, TriState::System);
        assert!((s.ai.per_run_budget_cny - 1.0).abs() < 1e-9);
    }

    #[test]
    fn old_blob_gains_new_fields() {
        // A minimal blob from a hypothetical older version.
        let s: Settings = serde_json::from_str(r#"{"practice":{"strict_typing":false}}"#).unwrap();
        assert!(!s.practice.strict_typing);
        assert!(s.practice.auto_speak_answer); // default filled in
        assert_eq!(s.sound.rate, 1.0);
    }
}
