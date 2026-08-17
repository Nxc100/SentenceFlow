//! channels.json — the serverless remote switch (spec §3.6).
//!
//! Ships with every content-pack rev. When free-model policy shifts, a CLI
//! release breaks compatibility, or a channel must be pulled, a content pack
//! update flips these bits without an app release.

use crate::meter::PriceTable;
use crate::types::ChannelId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const CURRENT_POLICY_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChannelPolicy {
    /// Schema version of this file.
    pub version: u32,
    /// Content-pack rev this policy shipped with.
    pub rev: u32,
    /// Per-channel enable bits; a missing channel counts as enabled.
    #[serde(default)]
    pub enabled: HashMap<ChannelId, bool>,
    /// Optional per-channel human note (通道卡预期管理, §10).
    #[serde(default)]
    pub notices: HashMap<ChannelId, String>,
    /// opencode versions with known breakage (substring match).
    #[serde(default)]
    pub opencode_known_bad: Vec<String>,
    /// 国内直连不可用、需代理的免费模型(id 子串匹配,大小写不敏感)。
    /// 面向国内用户的可达性标注 —— 上游路由会变,随内容包更新(§3.6)。
    /// 数据来源:真机直连实测(2026-08-17:仅 deepseek-v4-flash 系需代理)。
    #[serde(default)]
    pub proxy_required_models: Vec<String>,
    /// Fingerprint of the free-model list — change triggers re-benchmark
    /// (§3.5 名单变化自动重测).
    #[serde(default)]
    pub free_list_fingerprint: String,
    /// DeepSeek prices (user-editable copy seeds from this).
    pub deepseek_prices: PriceTable,
    /// Unix seconds the price table was last verified.
    pub prices_updated_at: i64,
    /// 微批大小 default (§3.6).
    pub default_microbatch: u32,
}

impl ChannelPolicy {
    pub fn from_json(json: &str) -> Result<Self, String> {
        let p: ChannelPolicy = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if p.version > CURRENT_POLICY_VERSION {
            return Err(format!(
                "channels.json version {} is newer than this app understands",
                p.version
            ));
        }
        Ok(p)
    }

    pub fn is_enabled(&self, id: ChannelId) -> bool {
        self.enabled.get(&id).copied().unwrap_or(true)
    }

    /// Days since the price table was verified — the CostBar shows a ⓘ once
    /// this grows stale (§6.4 价格表过旧).
    pub fn price_age_days(&self, now: i64) -> i64 {
        ((now - self.prices_updated_at).max(0)) / 86_400
    }

    /// 该模型在国内直连网络下是否需要代理(id 子串匹配,大小写不敏感)。
    pub fn model_needs_proxy(&self, model_id: &str) -> bool {
        let id = model_id.to_lowercase();
        self.proxy_required_models
            .iter()
            .any(|m| !m.is_empty() && id.contains(&m.to_lowercase()))
    }
}

impl Default for ChannelPolicy {
    /// Conservative built-in fallback used when the shipped file is missing
    /// or unparseable: everything enabled, prices must be user-confirmed.
    fn default() -> Self {
        Self {
            version: CURRENT_POLICY_VERSION,
            rev: 0,
            enabled: HashMap::new(),
            notices: HashMap::new(),
            opencode_known_bad: Vec::new(),
            // 内置兜底与随包 channels.json 保持一致(实测 2026-08-17)
            proxy_required_models: vec!["deepseek-v4-flash".into()],
            free_list_fingerprint: String::new(),
            deepseek_prices: PriceTable {
                prompt_per_m: 2.0,
                completion_per_m: 8.0,
            },
            prices_updated_at: 0,
            default_microbatch: 20,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
        "version": 1,
        "rev": 3,
        "enabled": {"zen": false},
        "notices": {"zen": "免费活动已结束"},
        "opencode_known_bad": ["1.18.2"],
        "free_list_fingerprint": "abc123",
        "deepseek_prices": {"prompt_per_m": 2.0, "completion_per_m": 8.0},
        "prices_updated_at": 1755000000,
        "default_microbatch": 20
    }"#;

    #[test]
    fn parses_and_answers_enable_bits() {
        let p = ChannelPolicy::from_json(SAMPLE).unwrap();
        assert!(!p.is_enabled(ChannelId::Zen));
        assert!(p.is_enabled(ChannelId::Opencode)); // missing = enabled
        assert_eq!(p.opencode_known_bad, vec!["1.18.2"]);
    }

    #[test]
    fn rejects_future_version() {
        let newer = SAMPLE.replace("\"version\": 1", "\"version\": 99");
        assert!(ChannelPolicy::from_json(&newer).is_err());
    }

    #[test]
    fn price_age() {
        let p = ChannelPolicy::from_json(SAMPLE).unwrap();
        assert_eq!(p.price_age_days(1_755_000_000 + 5 * 86_400), 5);
    }

    #[test]
    fn default_is_sane() {
        let p = ChannelPolicy::default();
        assert!(p.is_enabled(ChannelId::Deepseek));
        assert_eq!(p.default_microbatch, 20);
    }

    #[test]
    fn proxy_required_matches_by_substring_case_insensitive() {
        let p = ChannelPolicy::default();
        assert!(p.model_needs_proxy("opencode/deepseek-v4-flash-free"));
        assert!(p.model_needs_proxy("opencode/DeepSeek-V4-Flash"));
        // Zen 其余免费模型直连可用(用户真机实测)
        assert!(!p.model_needs_proxy("opencode/hy3-free"));
        assert!(!p.model_needs_proxy("opencode/big-pickle"));
        // DeepSeek 官方渠道自家模型不受此名单影响
        assert!(!p.model_needs_proxy("deepseek-chat"));
    }
}
