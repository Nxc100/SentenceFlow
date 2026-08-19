//! The four-channel trait (spec §3.4) — upper layers are channel-agnostic.

use crate::types::{ChannelError, ChannelStatus, ChatRequest, GenChunk, GenRequest, MeterKind};
use futures::stream::BoxStream;

#[async_trait::async_trait]
pub trait ChannelAdapter: Send + Sync {
    /// Non-destructive readiness probe (read-only, ≤3s, never touches
    /// credentials on disk — §11.C).
    async fn probe(&self) -> ChannelStatus;

    /// Stream one completion. Implementations map transport failures to
    /// [`ChannelError`]; they do **not** meter — metering wraps the stream at
    /// the orchestration layer so it is uniform across channels.
    async fn complete_stream(
        &self,
        req: GenRequest,
    ) -> Result<BoxStream<'static, Result<GenChunk, ChannelError>>, ChannelError>;

    /// Stream one multi-turn chat completion (AI 聊天模块). The default
    /// flattens the turns into a single-turn transcript so every channel
    /// works immediately; channels with native multi-turn support override
    /// it (OpenAI-compatible → messages array, opencode → `-s` server-side
    /// session, which additionally emits [`GenChunk::SessionRef`]).
    async fn chat_stream(
        &self,
        req: ChatRequest,
    ) -> Result<BoxStream<'static, Result<GenChunk, ChannelError>>, ChannelError> {
        self.complete_stream(req.into_single_turn()).await
    }

    /// How usage of this channel is metered/displayed (§5.5 CostBar).
    fn meter(&self) -> MeterKind;
}
