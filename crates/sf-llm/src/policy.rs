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
}
