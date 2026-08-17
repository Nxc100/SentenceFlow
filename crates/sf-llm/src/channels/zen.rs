//! Zen direct channel (spec §3.3): OpenAI-compatible endpoint with
//! `opencode/<model-id>` model names — free models without a local CLI.

use crate::adapter::ChannelAdapter;
use crate::channels::openai_compat::OpenAiCompatClient;
use crate::types::{ChannelError, ChannelStatus, GenChunk, GenRequest, MeterKind, ModelInfo};
use futures::stream::BoxStream;
use secrecy::SecretString;

pub const DEFAULT_BASE_URL: &str = "https://opencode.ai/zen/v1";

/// 内联条款小字 (§4.7): shown on every free Zen model.
pub const FREE_TERMS_NOTE: &str = "限时免费 · 限速 · 免费期数据或用于训练";

pub struct ZenChannel {
    client: OpenAiCompatClient,
    /// RPM estimate for the RateBudget CostBar form.
    rpm_estimate: u32,
}

impl ZenChannel {
    pub fn new(api_key: SecretString, rpm_estimate: u32, proxy: Option<String>) -> Self {
        Self::with_base_url(DEFAULT_BASE_URL, api_key, rpm_estimate, proxy)
    }

    pub fn with_base_url(
        base_url: impl Into<String>,
        api_key: SecretString,
        rpm_estimate: u32,
        proxy: Option<String>,
    ) -> Self {
        Self {
            client: OpenAiCompatClient::new(base_url, Some(api_key)).with_proxy(proxy),
            rpm_estimate,
        }
    }
}

#[async_trait::async_trait]
impl ChannelAdapter for ZenChannel {
    async fn probe(&self) -> ChannelStatus {
        match self.client.list_models().await {
            Ok(models) => ChannelStatus::Ready {
                models: models
                    .into_iter()
                    .map(|id| ModelInfo {
                        display_name: id.strip_prefix("opencode/").unwrap_or(&id).to_string(),
                        id,
                        terms_note: FREE_TERMS_NOTE.to_string(),
                        needs_proxy: false,
                    })
                    .collect(),
            },
            Err(ChannelError::BadKey) => ChannelStatus::NotAuthed,
            Err(e) => ChannelStatus::Error {
                message: e.zh_message(),
            },
        }
    }

    async fn complete_stream(
        &self,
        req: GenRequest,
    ) -> Result<BoxStream<'static, Result<GenChunk, ChannelError>>, ChannelError> {
        self.client.complete_stream(req).await
    }

    fn meter(&self) -> MeterKind {
        MeterKind::RateBudget {
            rpm_estimate: self.rpm_estimate,
        }
    }
}
