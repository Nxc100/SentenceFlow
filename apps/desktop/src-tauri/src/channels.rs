//! Channel registry: adapter construction, probing, key testing, bench
//! persistence (spec §4.7, §3.5).

use crate::error::{CmdError, CmdResult};
use crate::state::AppState;
use secrecy::SecretString;
use sf_llm::adapter::ChannelAdapter;
use sf_llm::channels::opencode::OpencodeConfig;
use sf_llm::channels::{DeepseekChannel, OllamaChannel, OpencodeChannel, ZenChannel};
use sf_llm::keystore;
use sf_llm::meter::PriceTable;
use sf_llm::types::{ChannelId, ChannelStatus};

pub fn effective_prices(state: &AppState) -> PriceTable {
    let settings = state.settings.lock().expect("settings lock");
    settings
        .ai
        .price_override
        .unwrap_or(state.policy.deepseek_prices)
}

/// Build an adapter for a channel. `key_override` lets [测试连接] try a key
/// before it is stored.
pub fn make_adapter(
    state: &AppState,
    channel: ChannelId,
    key_override: Option<SecretString>,
) -> CmdResult<Box<dyn ChannelAdapter>> {
    if !state.policy.is_enabled(channel) {
        let note = state
            .policy
            .notices
            .get(&channel)
            .cloned()
            .unwrap_or_else(|| "该通道已由内容包策略停用".into());
        return Err(CmdError::new("channel_disabled", note));
    }
    let stored_key = |ch: ChannelId| -> CmdResult<SecretString> {
        match key_override.clone() {
            Some(k) => Ok(k),
            None => keystore::load_key(ch)
                .map_err(|_| CmdError::new("no_key", "尚未配置 Key,请先在 AI 接入页填写")),
        }
    };
    // 用户配置的 AI 代理(§4.7 网络区):opencode 子进程经环境变量、HTTP
    // 通道经 reqwest Proxy;Ollama 本地永不代理。
    let proxy = {
        let settings = state.settings.lock().expect("settings lock");
        settings
            .ai
            .proxy_url
            .clone()
            .filter(|p| !p.trim().is_empty())
    };
    Ok(match channel {
        ChannelId::Opencode => {
            let settings = state.settings.lock().expect("settings lock");
            Box::new(OpencodeChannel::new(OpencodeConfig {
                bin_override: settings.ai.opencode_bin.clone().map(Into::into),
                sandbox_dir: state.paths.agent_sandbox(),
                known_bad_versions: state.policy.opencode_known_bad.clone(),
                rpm_estimate: 10,
                proxy_url: proxy,
            }))
        }
        ChannelId::Deepseek => Box::new(DeepseekChannel::new(
            stored_key(ChannelId::Deepseek)?,
            effective_prices(state),
            proxy,
        )),
        ChannelId::Zen => Box::new(ZenChannel::new(stored_key(ChannelId::Zen)?, 10, proxy)),
        ChannelId::Ollama => Box::new(OllamaChannel::default()),
    })
}

/// Probe one channel; missing keys map to `NotAuthed` (通道卡黄点) instead of
/// an error. Ready 时按 channels.json 名单回填每个模型的国内直连可达性
/// (需代理标注,面向国内用户 —— 可达性是策略,适配器不感知)。
pub async fn probe(state: &AppState, channel: ChannelId) -> ChannelStatus {
    let status = match make_adapter(state, channel, None) {
        Ok(adapter) => adapter.probe().await,
        Err(e) if e.code == "no_key" => return ChannelStatus::NotAuthed,
        Err(e) => return ChannelStatus::Error { message: e.message },
    };
    match status {
        // 名单只描述 Zen 网关背后的免费模型(opencode/Zen 两条通道);
        // DeepSeek 官方与 Ollama 本地天然直连可用。
        ChannelStatus::Ready { mut models }
            if matches!(channel, ChannelId::Opencode | ChannelId::Zen) =>
        {
            for m in &mut models {
                m.needs_proxy = state.policy.model_needs_proxy(&m.id);
            }
            ChannelStatus::Ready { models }
        }
        other => other,
    }
}

/// [测试连接]: probe with the pasted key; store it only on success (§6.3).
pub async fn test_and_store_key(
    state: &AppState,
    channel: ChannelId,
    key: SecretString,
) -> CmdResult<ChannelStatus> {
    let adapter = make_adapter(state, channel, Some(key.clone()))?;
    let status = adapter.probe().await;
    if matches!(status, ChannelStatus::Ready { .. }) {
        keystore::store_key(channel, &key).map_err(|e| CmdError::new("keystore", e.to_string()))?;
    }
    Ok(status)
}
