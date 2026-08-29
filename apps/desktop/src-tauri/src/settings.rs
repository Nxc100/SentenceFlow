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
    /// AI 萌宠。只由「AI 萌宠」页经 `pet_settings_set` 写入 —— 设置页的
    /// `set_settings` 会原样保留后端现值,避免两处写同一分节(见 commands::set_settings)。
    pub pet: PetSettings,
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

/// 主题。与 `packages/ui/src/tokens.css` 的 `[data-theme=…]` 一一对应,
/// 前端把它原样写进 `document.documentElement.dataset.theme`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    Light,
    Dark,
    /// 马卡龙少女:糖霜壁纸 + 玻璃卡面(与 `paper` 互斥,选中时护眼纸色不生效)。
    Macaron,
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
    /// 所选模型的展示名(选择时随手记下,供工坊顶栏等处显示;
    /// 面向小白用户不露 `opencode/hy3-free` 这类原始 id)。
    pub model_label: Option<String>,
    /// 单次预算上限,默认 ¥1 (§4.7).
    pub per_run_budget_cny: f64,
    /// 月度提醒阈值(None = 关).
    pub monthly_reminder_cny: Option<f64>,
    /// 用户手改价格表(None = 用 channels.json).
    pub price_override: Option<sf_llm::meter::PriceTable>,
    /// opencode 手动指定路径 (§6.4).
    pub opencode_bin: Option<String>,
    /// AI 通道 HTTP(S) 代理(如 `http://127.0.0.1:7890`)。直连网络访问
    /// opencode/Zen 境外端点时配置;DeepSeek 官方与 Ollama 本地不受影响。
    pub proxy_url: Option<String>,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            channel: None,
            model: None,
            model_label: None,
            per_run_budget_cny: 1.0,
            monthly_reminder_cny: None,
            price_override: None,
            opencode_bin: None,
            proxy_url: None,
        }
    }
}

/// AI 萌宠设置(《AI 萌宠模块-整合执行方案》§3.4)。
///
/// 与 HatchDesk 的 `config.json` 的差异,均为整合决策:
/// * 新增总开关 `enabled`,**默认关** —— 老用户升级后行为与基线完全一致,
///   宠物窗与心跳线程都不会起来,要主动到「AI 萌宠」页开启;
/// * `watcher_enabled` 默认改为 `false`(原 `true`):监听下载目录属隐私敏感面,
///   句流语境下保守化,功能保留可手动开;
/// * 丢弃 `pro`(句流免费,无付费判据)与 `analytics`(不做常驻埋点)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PetSettings {
    /// 总开关:关闭时不建宠物窗、不起心跳线程。
    pub enabled: bool,
    /// 宠物显示倍率(钳位 0.75–2.0)。
    pub scale: f32,
    /// 随机漫游。
    pub roam: bool,
    /// 全局点击穿透(与轮廓级命中测试互斥:开启后整窗不拦鼠标)。
    pub click_through: bool,
    /// 夜间自动睡觉(22:00–7:00)。
    pub night_sleep: bool,
    /// 省电模式:低帧率、关粒子。
    pub performance_mode: bool,
    /// 无操作多少分钟后入睡。
    pub sleep_after_min: u32,
    /// 道具叠加引擎(吃/提醒/睡的道具动画)。
    pub props_enabled: bool,
    /// 向导等待素材时监听系统下载目录(隐私敏感,默认关)。
    pub watcher_enabled: bool,
    /// ffmpeg 手动路径(默认自动探测:同目录 sidecar → PATH)。
    pub ffmpeg_path: Option<String>,
    /// 当前出场的宠物 ID。
    pub active_pet: Option<String>,
    /// 是否已完成首次孵化(只影响引导文案,不控制窗口显示)。
    pub first_hatch_done: bool,
}

impl Default for PetSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            scale: 1.0,
            roam: true,
            click_through: false,
            night_sleep: true,
            performance_mode: false,
            sleep_after_min: 8,
            props_enabled: true,
            watcher_enabled: false,
            ffmpeg_path: None,
            active_pet: None,
            first_hatch_done: false,
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
    fn theme_wire_names_match_css_data_theme() {
        // 线上值就是前端写进 data-theme 的字符串,改名等于换主题失效。
        for (theme, wire) in [
            (Theme::Light, "\"light\""),
            (Theme::Dark, "\"dark\""),
            (Theme::Macaron, "\"macaron\""),
            (Theme::System, "\"system\""),
        ] {
            assert_eq!(serde_json::to_string(&theme).unwrap(), wire);
            assert_eq!(serde_json::from_str::<Theme>(wire).unwrap(), theme);
        }
    }

    #[test]
    fn old_blob_gains_new_fields() {
        // A minimal blob from a hypothetical older version.
        let s: Settings = serde_json::from_str(r#"{"practice":{"strict_typing":false}}"#).unwrap();
        assert!(!s.practice.strict_typing);
        assert!(s.practice.auto_speak_answer); // default filled in
        assert_eq!(s.sound.rate, 1.0);
    }

    /// 「不影响既有功能」的最后一道闸门:老版本存下来的 settings 里没有
    /// `pet` 分节,反序列化后必须是**总开关关闭**的默认值 —— 升级后
    /// 既不会凭空冒出宠物窗,也不会起心跳线程。
    #[test]
    fn settings_without_pet_section_default_to_disabled() {
        let s: Settings =
            serde_json::from_str(r#"{"appearance":{"theme":"macaron"},"level":"L2"}"#).unwrap();
        assert_eq!(s.appearance.theme, Theme::Macaron, "既有分节照常读入");
        assert_eq!(s.level, Some(LevelId::L2));
        assert_eq!(s.pet, PetSettings::default());
        assert!(!s.pet.enabled, "老档升级后宠物必须是关的");
        assert!(!s.pet.watcher_enabled, "下载目录监听默认关(隐私保守化)");
        assert!(s.pet.roam && s.pet.night_sleep && s.pet.props_enabled);
    }

    /// pet 分节本身也要能增量演进:缺字段取默认,已有字段照读。
    #[test]
    fn pet_section_is_additively_migratable() {
        let s: Settings = serde_json::from_str(r#"{"pet":{"enabled":true,"scale":1.5}}"#).unwrap();
        assert!(s.pet.enabled);
        assert_eq!(s.pet.scale, 1.5);
        assert_eq!(s.pet.sleep_after_min, 8, "未出现的字段回落默认");
        assert_eq!(s.pet.active_pet, None);
    }

    /// 线上字段名即前端读写的键名,改名等于宠物设置整体失效。
    #[test]
    fn pet_wire_names_are_snake_case() {
        let json = serde_json::to_value(Settings::default()).unwrap();
        let pet = &json["pet"];
        for key in [
            "enabled",
            "scale",
            "roam",
            "click_through",
            "night_sleep",
            "performance_mode",
            "sleep_after_min",
            "props_enabled",
            "watcher_enabled",
            "ffmpeg_path",
            "active_pet",
            "first_hatch_done",
        ] {
            assert!(
                !pet[key].is_null() || pet.get(key).is_some(),
                "缺少字段 {key}"
            );
        }
    }
}
