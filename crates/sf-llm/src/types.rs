//! Shared channel types (spec §3.4).

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelId {
    Opencode,
    Deepseek,
    Zen,
    Ollama,
}

impl ChannelId {
    pub const ALL: [ChannelId; 4] = [
        ChannelId::Opencode,
        ChannelId::Deepseek,
        ChannelId::Zen,
        ChannelId::Ollama,
    ];

    pub fn display_name(&self) -> &'static str {
        match self {
            ChannelId::Opencode => "opencode 本地",
            ChannelId::Deepseek => "DeepSeek 官方",
            ChannelId::Zen => "Zen 直连",
            ChannelId::Ollama => "Ollama 本地",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInfo {
    /// Channel-native model id (e.g. `opencode/deepseek-v4-flash`).
    pub id: String,
    pub display_name: String,
    /// 免费模型条款标注 (§4.7): e.g. "限时免费 · 数据或用于训练".
    #[serde(default)]
    pub terms_note: String,
    /// 国内直连不可用、需代理(channels.json 名单标注,面向国内用户)。
    /// 适配器产出时恒为 false,由编排层按 [`crate::policy::ChannelPolicy`]
    /// 回填 —— 可达性是策略,不是传输属性。
    #[serde(default)]
    pub needs_proxy: bool,
}

/// Result of probing a channel (通道卡三态 + 错误, §4.7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ChannelStatus {
    /// CLI/runtime not found (opencode 未安装 / Ollama 未启动).
    NotInstalled,
    /// Installed but not authenticated (opencode 未登录 / Key 未配置).
    NotAuthed,
    /// Ready with its usable model list.
    Ready { models: Vec<ModelInfo> },
    /// Probe failed for another reason (human-readable, already 人话).
    Error { message: String },
}

/// One generation request handed to an adapter. Prompt assembly happens in
/// sf-pipeline; adapters only transport.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GenRequest {
    pub model: String,
    pub system: String,
    pub user: String,
    /// Hard output cap the adapter passes to the backend when supported.
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
}

/// Streaming chunk from an adapter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GenChunk {
    /// Incremental output text.
    Text { text: String },
    /// Final usage numbers if the backend reports them (usage 回填, §7.5).
    Usage {
        prompt_tokens: u64,
        completion_tokens: u64,
    },
    /// Channel-native conversation handle for continuing later (opencode
    /// `run -s <id>` server-side memory). Emitted at most once per stream;
    /// channels without native sessions never emit it.
    SessionRef { id: String },
    /// Stream finished normally.
    Done,
}

/// Speaker of one prior conversation turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    User,
    Assistant,
}

/// One turn of a multi-turn conversation, oldest first in
/// [`ChatRequest::turns`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatTurn {
    pub role: ChatRole,
    pub text: String,
}

/// One multi-turn chat request (AI 聊天模块). `turns` always carries the full
/// visible context ending with the new user message; channels with native
/// session memory may ignore everything but the tail (see the opencode
/// adapter), the rest replay it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatRequest {
    pub model: String,
    pub system: String,
    /// Full conversation, oldest first; the last entry must be the new user
    /// message.
    pub turns: Vec<ChatTurn>,
    /// Channel-native session to continue (from an earlier
    /// [`GenChunk::SessionRef`]). `None` starts a fresh conversation.
    pub session: Option<String>,
    /// Instructions that must hold for **this turn** and cannot be assumed
    /// to persist server-side. Channels that resend `system` every turn
    /// (the OpenAI-compatible ones) ignore it — it is already in `system`.
    /// Channels that only send `system` when the session is created
    /// (opencode `-s`) prepend it to each continued message, so toggling a
    /// per-turn option mid-conversation actually takes effect.
    #[serde(default)]
    pub per_turn: String,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
}

impl ChatRequest {
    /// Flatten into a single-turn [`GenRequest`] — the default path for
    /// channels without native multi-turn support. Earlier turns become a
    /// labelled transcript above the new message.
    pub fn into_single_turn(self) -> GenRequest {
        GenRequest {
            model: self.model,
            system: self.system,
            user: flatten_turns(self.turns),
            max_tokens: self.max_tokens,
            temperature: self.temperature,
        }
    }
}

/// Pack turns into one user prompt: bare passthrough for a single user turn,
/// transcript + new-message sections otherwise.
pub fn flatten_turns(mut turns: Vec<ChatTurn>) -> String {
    let last = match turns.last() {
        Some(t) if t.role == ChatRole::User => turns.pop().map(|t| t.text).unwrap_or_default(),
        _ => String::new(), // defensive: contract says last turn is the user's
    };
    if turns.is_empty() {
        return last;
    }
    let mut s = String::from("[Conversation so far]\n");
    for t in &turns {
        let label = match t.role {
            ChatRole::User => "User",
            ChatRole::Assistant => "Assistant",
        };
        s.push_str(label);
        s.push_str(": ");
        s.push_str(&t.text);
        s.push('\n');
    }
    s.push_str("\n[New user message]\n");
    s.push_str(&last);
    s
}

/// Channel errors, mapped to 人话 by [`ChannelError::zh_message`] (§11.E).
#[derive(Debug, Error)]
pub enum ChannelError {
    #[error("channel binary not found")]
    NotInstalled,
    #[error("not authenticated")]
    NotAuthed,
    #[error("invalid API key")]
    BadKey,
    #[error("insufficient balance")]
    InsufficientBalance,
    #[error("rate limited, retry after {retry_after_secs}s")]
    RateLimited { retry_after_secs: u64 },
    #[error("request timed out")]
    Timeout,
    #[error("budget exhausted mid-stream")]
    BudgetExhausted,
    #[error("network error: {0}")]
    Network(String),
    #[error("backend error {status}: {message}")]
    Backend { status: u16, message: String },
    #[error("process error: {0}")]
    Process(String),
}

impl ChannelError {
    /// 错误码人话对照 (§11.E).
    pub fn zh_message(&self) -> String {
        match self {
            ChannelError::NotInstalled => "未找到 opencode,可复制下方命令安装或手动指定路径".into(),
            ChannelError::NotAuthed => "尚未登录,请在终端运行 opencode auth login".into(),
            ChannelError::BadKey => "Key 无效,请检查是否复制完整".into(),
            ChannelError::InsufficientBalance => "账户余额不足".into(),
            ChannelError::RateLimited { retry_after_secs } => {
                format!("触发限速,{retry_after_secs}s 后自动重试")
            }
            ChannelError::Timeout => "网络不稳定,已停表,可重试".into(),
            ChannelError::BudgetExhausted => "已达单次预算上限,已优雅停止".into(),
            ChannelError::Network(_) => "网络异常,请检查连接后重试".into(),
            ChannelError::Backend { status, .. } => match status {
                401 => "Key 无效,请检查是否复制完整".into(),
                402 => "账户余额不足".into(),
                429 => "触发限速,稍后自动重试".into(),
                s => format!("服务端错误({s}),可稍后重试"),
            },
            ChannelError::Process(_) => "本地进程启动失败,可查看日志或手动指定路径".into(),
        }
    }
}

/// What the CostBar shows for a channel (§5.5 双形态).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MeterKind {
    /// 付费形态: per-token prices (per 1M tokens, CNY).
    Money {
        prompt_price_per_m: f64,
        completion_price_per_m: f64,
    },
    /// 免费形态: request budget with estimated RPM cap.
    RateBudget { rpm_estimate: u32 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_maps_to_human_chinese() {
        assert!(ChannelError::BadKey.zh_message().contains("复制完整"));
        assert!(
            ChannelError::Backend {
                status: 402,
                message: String::new()
            }
            .zh_message()
            .contains("余额")
        );
        assert!(
            ChannelError::RateLimited {
                retry_after_secs: 8
            }
            .zh_message()
            .contains("8s")
        );
    }

    #[test]
    fn status_serializes_tagged() {
        let s = ChannelStatus::Ready { models: vec![] };
        assert_eq!(
            serde_json::to_string(&s).unwrap(),
            r#"{"state":"ready","models":[]}"#
        );
    }

    #[test]
    fn session_ref_serializes_tagged() {
        let c = GenChunk::SessionRef { id: "ses_1".into() };
        assert_eq!(
            serde_json::to_string(&c).unwrap(),
            r#"{"kind":"session_ref","id":"ses_1"}"#
        );
    }

    fn turn(role: ChatRole, text: &str) -> ChatTurn {
        ChatTurn {
            role,
            text: text.into(),
        }
    }

    #[test]
    fn single_user_turn_flattens_bare() {
        assert_eq!(
            flatten_turns(vec![turn(ChatRole::User, "Hi there")]),
            "Hi there"
        );
    }

    #[test]
    fn multi_turn_flattens_to_transcript() {
        let s = flatten_turns(vec![
            turn(ChatRole::User, "Hello"),
            turn(ChatRole::Assistant, "Hi! How are you?"),
            turn(ChatRole::User, "I am fine"),
        ]);
        assert!(s.starts_with("[Conversation so far]\n"));
        assert!(s.contains("User: Hello\n"));
        assert!(s.contains("Assistant: Hi! How are you?\n"));
        assert!(s.ends_with("[New user message]\nI am fine"));
        // the new message must not appear inside the transcript section
        assert_eq!(s.matches("I am fine").count(), 1);
    }

    #[test]
    fn into_single_turn_carries_request_fields() {
        let req = ChatRequest {
            model: "m".into(),
            system: "sys".into(),
            turns: vec![turn(ChatRole::User, "hey")],
            session: Some("ses_x".into()),
            per_turn: String::new(),
            max_tokens: Some(64),
            temperature: Some(0.5),
        };
        let single = req.into_single_turn();
        assert_eq!(single.model, "m");
        assert_eq!(single.system, "sys");
        assert_eq!(single.user, "hey");
        assert_eq!(single.max_tokens, Some(64));
        assert_eq!(single.temperature, Some(0.5));
    }
}
