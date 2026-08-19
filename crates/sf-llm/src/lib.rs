//! # sf-llm — four-channel LLM access (spec §3, §7.5)
//!
//! One trait ([`ChannelAdapter`]) over four transports:
//! * **opencode local** — drives the user's own opencode CLI (serve preferred,
//!   run fallback);
//! * **DeepSeek** — official HTTPS API (user key);
//! * **Zen direct** — OpenAI-compatible endpoint (user key);
//! * **Ollama** — localhost:11434, fully offline.
//!
//! Guard rails are structural, not aspirational:
//! * dual-track metering: [`meter::MoneyMeter`] (estimate → live count → hard
//!   budget stop) and [`meter::RateBudget`] (client token bucket + visible
//!   exponential backoff);
//! * keys live in the OS keychain via [`keystore`], held as `SecretString`,
//!   never logged;
//! * the practice path never links this crate — every request originates from
//!   生成工坊 / 答疑 / 周点评 only (spec §7.5).

pub mod adapter;
pub mod backoff;
pub mod bench;
pub mod channels;
pub mod estimate;
pub mod meter;
pub mod policy;
pub mod queue;
pub mod types;

#[cfg(not(target_family = "wasm"))]
pub mod keystore;

pub use adapter::ChannelAdapter;
pub use types::{
    ChannelError, ChannelId, ChannelStatus, ChatRequest, ChatRole, ChatTurn, GenChunk, GenRequest,
    MeterKind, ModelInfo,
};
