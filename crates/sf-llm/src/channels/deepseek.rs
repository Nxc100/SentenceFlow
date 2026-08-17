//! DeepSeek official API channel (spec §3.3) — 质量与稳定基准.

use crate::adapter::ChannelAdapter;
use crate::channels::openai_compat::OpenAiCompatClient;
use crate::meter::PriceTable;
use crate::types::{ChannelError, ChannelStatus, GenChunk, GenRequest, MeterKind, ModelInfo};
use futures::stream::BoxStream;
use secrecy::SecretString;

pub const DEFAULT_BASE_URL: &str = "https://api.deepseek.com/v1";

pub struct DeepseekChannel {
    client: OpenAiCompatClient,
    prices: PriceTable,
}

impl DeepseekChannel {
    pub fn new(api_key: SecretString, prices: PriceTable, proxy: Option<String>) -> Self {
        Self::with_base_url(DEFAULT_BASE_URL, api_key, prices, proxy)
    }

    /// Base URL override — used by tests and by channels.json migrations.
    pub fn with_base_url(
        base_url: impl Into<String>,
        api_key: SecretString,
        prices: PriceTable,
        proxy: Option<String>,
    ) -> Self {
        Self {
            client: OpenAiCompatClient::new(base_url, Some(api_key)).with_proxy(proxy),
            prices,
        }
    }
}

#[async_trait::async_trait]
impl ChannelAdapter for DeepseekChannel {
    async fn probe(&self) -> ChannelStatus {
        match self.client.list_models().await {
            Ok(models) => ChannelStatus::Ready {
                models: models
                    .into_iter()
                    .map(|id| ModelInfo {
                        display_name: id.clone(),
                        id,
                        terms_note: String::new(),
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
        MeterKind::Money {
            prompt_price_per_m: self.prices.prompt_per_m,
            completion_price_per_m: self.prices.completion_per_m,
        }
    }
}
