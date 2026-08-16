//! The four-channel trait (spec §3.4) — upper layers are channel-agnostic.

use crate::types::{ChannelError, ChannelStatus, GenChunk, GenRequest, MeterKind};
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

    /// How usage of this channel is metered/displayed (§5.5 CostBar).
    fn meter(&self) -> MeterKind;
}
