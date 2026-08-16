//! Channel adapter implementations.

pub mod deepseek;
pub mod ollama;
pub mod openai_compat;
pub mod opencode;
pub mod sse;
pub mod zen;

pub use deepseek::DeepseekChannel;
pub use ollama::OllamaChannel;
pub use opencode::OpencodeChannel;
pub use zen::ZenChannel;
