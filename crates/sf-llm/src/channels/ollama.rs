//! Ollama local channel (spec §3.3) — 全离线兜底, localhost:11434.
//!
//! Uses Ollama's OpenAI-compatible `/v1` endpoints for completions and its
//! native `/api/tags` for the installed-models probe (红点 + 如何安装, §4.7).

use crate::adapter::ChannelAdapter;
use crate::channels::openai_compat::OpenAiCompatClient;
use crate::types::{ChannelError, ChannelStatus, GenChunk, GenRequest, MeterKind, ModelInfo};
use futures::stream::BoxStream;
use serde::Deserialize;
use std::time::Duration;

pub const DEFAULT_BASE_URL: &str = "http://127.0.0.1:11434";

pub struct OllamaChannel {
    base_url: String,
}

impl Default for OllamaChannel {
    fn default() -> Self {
        Self::new(DEFAULT_BASE_URL)
    }
}

impl OllamaChannel {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
        }
    }

    fn compat(&self) -> OpenAiCompatClient {
        // localhost 永不走代理:环境/系统代理开着也不受影响
        OpenAiCompatClient::new(format!("{}/v1", self.base_url), None).without_proxy()
    }
}

#[async_trait::async_trait]
impl ChannelAdapter for OllamaChannel {
    async fn probe(&self) -> ChannelStatus {
        #[derive(Deserialize)]
        struct Tags {
            #[serde(default)]
            models: Vec<Tag>,
        }
        #[derive(Deserialize)]
        struct Tag {
            name: String,
        }
        let client = match reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(3))
            // localhost 探测不经任何代理(环境代理开着也不受影响)
            .no_proxy()
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                return ChannelStatus::Error {
                    message: e.to_string(),
                };
            }
        };
        match client
            .get(format!("{}/api/tags", self.base_url))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => match resp.json::<Tags>().await {
                Ok(tags) => ChannelStatus::Ready {
                    models: tags
                        .models
                        .into_iter()
                        .map(|t| ModelInfo {
                            display_name: t.name.clone(),
                            id: t.name,
                            terms_note: String::new(),
                        })
                        .collect(),
                },
                Err(e) => ChannelStatus::Error {
                    message: e.to_string(),
                },
            },
            Ok(resp) => ChannelStatus::Error {
                message: format!("Ollama 响应异常({})", resp.status()),
            },
            // Connection refused ⇒ not running: 未检测到本地服务(11434).
            Err(_) => ChannelStatus::NotInstalled,
        }
    }

    async fn complete_stream(
        &self,
        req: GenRequest,
    ) -> Result<BoxStream<'static, Result<GenChunk, ChannelError>>, ChannelError> {
        self.compat().complete_stream(req).await
    }

    fn meter(&self) -> MeterKind {
        // Local inference is free and unmetered; a generous bucket keeps the
        // CostBar's free form meaningful without ever throttling in practice.
        MeterKind::RateBudget { rpm_estimate: 600 }
    }
}
