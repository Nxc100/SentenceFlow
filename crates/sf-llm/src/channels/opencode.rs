//! opencode local channel (spec §3.4, §11.A/C) — the zero-cost geek channel.
//!
//! Mode strategy — **standalone `opencode run --format json` per request**.
//! The W2 on-machine spike (opencode 1.18.18) found that a `run --attach`
//! against a password-protected `serve` instance emits only `step_start` and
//! never relays `text` events to the attached process, so the doc's
//! serve-first preference (§3.4) is not implementable through the current CLI
//! surface. Per the §10 escape-hatch posture we take the documented fallback:
//! cold-start `run` costs ~1-2s per micro-batch and behaves identically
//! otherwise. Revisit via the official `@opencode-ai/sdk` session API when
//! serve integration is next attempted (recorded in doc/开发状态.md).
//!
//! Verified event shapes (1.18.18): one JSON object per stdout line —
//! `{"type":"step_start"…}`, `{"type":"text","part":{"text":"…"}}`,
//! `{"type":"step_finish"…}`; [`run_line_to_chunk`] parses tolerantly.
//!
//! 隐私红线: probing never opens `auth.json` — login state is inferred only
//! from `opencode models` output (§11.C). All probe steps are read-only with
//! a 3s timeout.

use crate::adapter::ChannelAdapter;
use crate::types::{
    ChannelError, ChannelStatus, ChatRequest, GenChunk, GenRequest, MeterKind, ModelInfo,
    flatten_turns,
};
use futures::StreamExt;
use futures::stream::BoxStream;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

/// 探测超时(有意偏离 §11.C 的"3s":opencode 是 ~179MB 的 Bun 单文件,
/// 慢磁盘/虚拟机上冷启动就要 3–10s,3s 会把"程序还在加载"误判成网络故障
/// ——真机踩坑,见 doc/开发状态.md)。探测仍然全程只读。
const PROBE_VERSION_TIMEOUT: Duration = Duration::from_secs(20);
/// 模型名单可能触发首次联网拉取,给更宽裕的窗口。
const PROBE_MODELS_TIMEOUT: Duration = Duration::from_secs(45);

/// 我方托管的沙箱 `opencode.json` (§3.4 约束②; 键名 W2 spike 对官方文档钉死).
/// The desktop app writes this file into the sandbox dir on every update.
pub const SANDBOX_OPENCODE_JSON: &str = r#"{
  "agent": {
    "sf-gen": {
      "description": "SentenceFlow generation-only agent",
      "prompt": "只输出符合 schema 的 JSON 数组,不使用任何工具。",
      "permission": { "edit": "deny", "bash": "deny", "webfetch": "deny" }
    }
  }
}
"#;

/// 安装引导用的内联命令 (§4.7 通道卡).
pub const INSTALL_COMMAND: &str = "curl -fsSL https://opencode.ai/install | bash";
pub const LOGIN_COMMAND: &str = "opencode auth login";

#[derive(Debug, Clone)]
pub struct OpencodeConfig {
    /// 手动指定二进制路径 (§6.4 失败态入口).
    pub bin_override: Option<PathBuf>,
    /// Empty sandbox directory holding our `opencode.json` (§7.7).
    pub sandbox_dir: PathBuf,
    /// known-bad versions from channels.json (§3.6).
    pub known_bad_versions: Vec<String>,
    /// RPM estimate for the free-form CostBar.
    pub rpm_estimate: u32,
    /// HTTP(S) 代理传给 CLI 子进程(HTTP_PROXY/HTTPS_PROXY 环境变量;
    /// Bun/Node 的 fetch 均遵守)。直连网络访问 Zen 端点需要它(§4.7 网络区)。
    pub proxy_url: Option<String>,
}

pub struct OpencodeChannel {
    cfg: OpencodeConfig,
}

impl OpencodeChannel {
    pub fn new(cfg: OpencodeConfig) -> Self {
        Self { cfg }
    }

    /// Locate the opencode binary: override → PATH → common install dirs.
    pub fn find_binary(&self) -> Option<PathBuf> {
        locate_binary(self.cfg.bin_override.as_deref())
    }

    /// [`hidden_command`] with this channel's proxy applied.
    fn command(&self, bin: &Path) -> Command {
        hidden_command(bin, self.cfg.proxy_url.as_deref())
    }

    /// Spawn one sandboxed `opencode run --format json` with the prompt on
    /// stdin — standalone run per request (see module docs for why `--attach`
    /// is deliberately not used). `session` continues a server-side session.
    fn spawn_run(
        &self,
        model: &str,
        session: Option<&str>,
        prompt: &str,
    ) -> Result<tokio::process::Child, ChannelError> {
        let bin = self.find_binary().ok_or(ChannelError::NotInstalled)?;
        std::fs::create_dir_all(&self.cfg.sandbox_dir)
            .map_err(|e| ChannelError::Process(e.to_string()))?;
        std::fs::write(
            self.cfg.sandbox_dir.join("opencode.json"),
            SANDBOX_OPENCODE_JSON,
        )
        .map_err(|e| ChannelError::Process(e.to_string()))?;

        let mut cmd = self.command(&bin);
        cmd.args(["run", "-m", model, "--format", "json"]);
        if let Some(id) = session {
            cmd.args(["-s", id]);
        }
        let mut child = cmd
            .current_dir(&self.cfg.sandbox_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| ChannelError::Process(e.to_string()))?;

        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| ChannelError::Process("failed to open opencode stdin".into()))?;
        let prompt = prompt.to_string();
        tokio::spawn(async move {
            let _ = stdin.write_all(prompt.as_bytes()).await;
            let _ = stdin.shutdown().await;
        });
        Ok(child)
    }

    async fn run_capture(
        &self,
        bin: &Path,
        args: &[&str],
        timeout: Duration,
    ) -> Result<String, ChannelError> {
        let mut cmd = self.command(bin);
        let fut = cmd
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output();
        let out = tokio::time::timeout(timeout, fut)
            .await
            .map_err(|_| ChannelError::Timeout)?
            .map_err(|e| ChannelError::Process(e.to_string()))?;
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

/// Locate an opencode binary: override → PATH → common install dirs.
/// Shared with the desktop agent shell (智能体模式), which drives the CLI
/// directly rather than through the adapter.
pub fn locate_binary(bin_override: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = bin_override {
        return p.exists().then(|| p.to_path_buf());
    }
    // PATH lookup.
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            for name in binary_names() {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    // Common fallback locations (§10: 用户环境差异对策).
    for dir in common_install_dirs() {
        for name in binary_names() {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Windows 下 GUI 进程 spawn `.cmd`(经 cmd.exe)会弹出黑色控制台窗口;
/// `CREATE_NO_WINDOW` 让子进程静默运行。可选代理注入环境变量
/// (HTTP_PROXY/HTTPS_PROXY;localhost 排除)。与桌面智能体外壳共用。
pub fn hidden_command(bin: &Path, proxy: Option<&str>) -> Command {
    let mut cmd = Command::new(bin);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    if let Some(proxy) = proxy.map(str::trim)
        && !proxy.is_empty()
    {
        cmd.env("HTTP_PROXY", proxy)
            .env("HTTPS_PROXY", proxy)
            .env("NO_PROXY", "localhost,127.0.0.1");
    }
    cmd
}

#[async_trait::async_trait]
impl ChannelAdapter for OpencodeChannel {
    async fn probe(&self) -> ChannelStatus {
        // ① binary present?
        let Some(bin) = self.find_binary() else {
            return ChannelStatus::NotInstalled;
        };
        // ② version baseline / known-bad (§11.C).
        match self
            .run_capture(&bin, &["--version"], PROBE_VERSION_TIMEOUT)
            .await
        {
            Ok(v) => {
                let version = v.trim().to_string();
                if self
                    .cfg
                    .known_bad_versions
                    .iter()
                    .any(|bad| version.contains(bad.as_str()))
                {
                    return ChannelStatus::Error {
                        message: format!("此 opencode 版本({version})存在已知问题,通道已禁用"),
                    };
                }
            }
            Err(ChannelError::Timeout) => {
                // 不是网络问题:大概率是大体积二进制在慢磁盘上冷启动
                return ChannelStatus::Error {
                    message: "opencode 启动较慢(首次运行或磁盘较忙)——稍等片刻再点[重新检测]".into(),
                };
            }
            Err(e) => {
                return ChannelStatus::Error {
                    message: e.zh_message(),
                };
            }
        }
        // ③ 可用性只看 `opencode models` 输出(从不读 auth.json)。
        //    实证修正(2026-08-18,隔离 auth 的全新环境真实生成通过):
        //    Zen 免费层匿名可用、无需登录——名单为空更可能是网络波动或
        //    名单下线;NotAuthed 在 UI 层呈现为「重试 + 备用登录」,
        //    而非把登录当成必经步骤。
        match self
            .run_capture(&bin, &["models"], PROBE_MODELS_TIMEOUT)
            .await
        {
            Ok(out) => {
                let models = parse_models_output(&out);
                if models.is_empty() {
                    ChannelStatus::NotAuthed
                } else {
                    ChannelStatus::Ready { models }
                }
            }
            Err(ChannelError::Timeout) => ChannelStatus::NotAuthed,
            Err(e) => ChannelStatus::Error {
                message: e.zh_message(),
            },
        }
    }

    async fn complete_stream(
        &self,
        req: GenRequest,
    ) -> Result<BoxStream<'static, Result<GenChunk, ChannelError>>, ChannelError> {
        // Prompt goes through stdin (§3.4): system + user with a separator.
        let prompt = format!("{}\n\n---\n\n{}", req.system, req.user);
        let child = self.spawn_run(&req.model, None, &prompt)?;
        Ok(stream_child_stdout(child, |line| {
            run_line_to_chunk(line).into_iter().collect()
        })?)
    }

    /// Multi-turn chat via server-side session memory (`run -s <id>`,实机
    /// spike 2026-08-19 验证跨请求记忆可用)。新会话把 system + 上下文塞进
    /// 首条消息并从事件流捕获 sessionID(经 [`GenChunk::SessionRef`] 回传);
    /// 续聊只送最新用户消息 —— 上文在 opencode 服务端。
    async fn chat_stream(
        &self,
        req: ChatRequest,
    ) -> Result<BoxStream<'static, Result<GenChunk, ChannelError>>, ChannelError> {
        let prompt = build_chat_prompt(&req);
        let child = self.spawn_run(&req.model, req.session.as_deref(), &prompt)?;
        let mut session_seen = false;
        Ok(stream_child_stdout(child, move |line| {
            chat_line_to_chunks(line, &mut session_seen)
        })?)
    }

    fn meter(&self) -> MeterKind {
        MeterKind::RateBudget {
            rpm_estimate: self.cfg.rpm_estimate,
        }
    }
}

fn binary_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["opencode.exe", "opencode.cmd", "opencode"]
    } else {
        &["opencode"]
    }
}

fn common_install_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        let home = PathBuf::from(home);
        dirs.push(home.join(".local").join("bin"));
        dirs.push(home.join(".opencode").join("bin"));
        dirs.push(home.join("bin"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let local = PathBuf::from(local);
        dirs.push(local.join("Programs").join("opencode"));
        dirs.push(local.join("opencode"));
    }
    dirs.push(PathBuf::from("/usr/local/bin"));
    dirs.push(PathBuf::from("/opt/homebrew/bin"));
    dirs
}

/// Parse `opencode models` output: one model id per line; channel usability =
/// "has at least one `opencode/` entry"(§11.C;免费层匿名可用,与登录无关)。
pub fn parse_models_output(out: &str) -> Vec<ModelInfo> {
    out.lines()
        .map(str::trim)
        .filter(|l| l.starts_with("opencode/"))
        .map(|id| ModelInfo {
            display_name: id.strip_prefix("opencode/").unwrap_or(id).to_string(),
            id: id.to_string(),
            terms_note: crate::channels::zen::FREE_TERMS_NOTE.to_string(),
            needs_proxy: false,
        })
        .collect()
}

/// 组装喂给 `opencode run` 的提示词(纯函数,可测)。
///
/// * 新会话:system + 本地历史一起送,给服务端会话打底;
/// * 续聊:上文在服务端,只送最新用户消息 —— 但 `per_turn` 必须随行,
///   因为 system 不会重发,用户中途改的开关否则不生效(真机踩坑)。
pub fn build_chat_prompt(req: &ChatRequest) -> String {
    match &req.session {
        Some(_) => {
            let last = req.turns.last().map(|t| t.text.as_str()).unwrap_or("");
            let rules = req.per_turn.trim();
            if rules.is_empty() {
                last.to_string()
            } else {
                format!("{rules}\n\n---\n\n{last}")
            }
        }
        None => format!(
            "{}\n\n---\n\n{}",
            req.system,
            flatten_turns(req.turns.clone())
        ),
    }
}

/// Turn a spawned child's stdout lines into a chunk stream: each line maps to
/// zero or more chunks via `map_line`; stdout close reaps the child (non-zero
/// exit → error) and finishes with [`GenChunk::Done`].
fn stream_child_stdout(
    mut child: tokio::process::Child,
    map_line: impl FnMut(&str) -> Vec<GenChunk> + Send + 'static,
) -> Result<BoxStream<'static, Result<GenChunk, ChannelError>>, ChannelError> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ChannelError::Process("failed to open opencode stdout".into()))?;
    let lines = BufReader::new(stdout).lines();
    let pending: VecDeque<GenChunk> = VecDeque::new();
    let stream = futures::stream::unfold(
        (lines, Some(child), pending, map_line),
        |(mut lines, mut child, mut pending, mut map_line)| async move {
            loop {
                if let Some(chunk) = pending.pop_front() {
                    return Some((Ok(chunk), (lines, child, pending, map_line)));
                }
                match lines.next_line().await {
                    Ok(Some(line)) => pending.extend(map_line(&line)),
                    Ok(None) => {
                        // stdout closed: reap the child and finish.
                        if let Some(c) = child.as_mut() {
                            match c.wait().await {
                                Ok(status) if !status.success() => {
                                    let err = ChannelError::Process(format!(
                                        "opencode run exited with {status}"
                                    ));
                                    return Some((Err(err), (lines, None, pending, map_line)));
                                }
                                _ => {}
                            }
                            return Some((Ok(GenChunk::Done), (lines, None, pending, map_line)));
                        }
                        return None;
                    }
                    Err(e) => {
                        return Some((
                            Err(ChannelError::Process(e.to_string())),
                            (lines, None, pending, map_line),
                        ));
                    }
                }
            }
        },
    );
    Ok(stream.boxed())
}

/// Map one chat-mode stdout line to chunks: first sighting of the top-level
/// `sessionID` (present on every event, W3 spike) becomes a
/// [`GenChunk::SessionRef`], then normal text mapping applies.
pub fn chat_line_to_chunks(line: &str, session_seen: &mut bool) -> Vec<GenChunk> {
    let mut out = Vec::new();
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return out;
    }
    if !*session_seen
        && let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed)
        && let Some(id) = v.get("sessionID").and_then(|s| s.as_str())
        && !id.is_empty()
    {
        *session_seen = true;
        out.push(GenChunk::SessionRef { id: id.to_string() });
    }
    out.extend(run_line_to_chunk(line));
    out
}

/// Map one `opencode run --format json` stdout line to a chunk.
///
/// The exact event schema is pinned against the official docs in the W2
/// spike; parsing is deliberately tolerant: JSON lines with a recognizable
/// text field stream as text, non-JSON lines stream verbatim, structural
/// events are skipped.
pub fn run_line_to_chunk(line: &str) -> Option<GenChunk> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(v) => {
            for key in ["text", "content", "delta"] {
                if let Some(t) = v.get(key).and_then(|t| t.as_str())
                    && !t.is_empty()
                {
                    return Some(GenChunk::Text {
                        text: t.to_string(),
                    });
                }
            }
            // part-based event shape: {"type":"text","part":{"text":"…"}}
            if let Some(t) = v.pointer("/part/text").and_then(|t| t.as_str())
                && !t.is_empty()
            {
                return Some(GenChunk::Text {
                    text: t.to_string(),
                });
            }
            None // structural event (step start/finish etc.)
        }
        // Plain text output (older CLIs / --format text fallback).
        Err(_) => Some(GenChunk::Text {
            text: format!("{trimmed}\n"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_output_parses_and_infers_login() {
        let out = "anthropic/claude-sonnet-5\nopencode/deepseek-v4-flash\nopencode/big-pickle\n";
        let models = parse_models_output(out);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "opencode/deepseek-v4-flash");
        assert_eq!(models[0].display_name, "deepseek-v4-flash");
        assert!(parse_models_output("no models\n").is_empty());
    }

    #[test]
    fn run_lines_map_to_chunks() {
        assert_eq!(
            run_line_to_chunk(r#"{"type":"text","text":"Hello"}"#),
            Some(GenChunk::Text {
                text: "Hello".into()
            })
        );
        assert_eq!(
            run_line_to_chunk(r#"{"type":"part","part":{"text":"Hi"}}"#),
            Some(GenChunk::Text { text: "Hi".into() })
        );
        assert_eq!(run_line_to_chunk(r#"{"type":"step-start"}"#), None);
        assert_eq!(run_line_to_chunk(""), None);
        assert_eq!(
            run_line_to_chunk("plain output"),
            Some(GenChunk::Text {
                text: "plain output\n".into()
            })
        );
    }

    #[test]
    fn chat_lines_emit_session_ref_once_then_text() {
        let mut seen = false;
        let first = chat_line_to_chunks(
            r#"{"type":"step_start","sessionID":"ses_abc","part":{"messageID":"msg_1"}}"#,
            &mut seen,
        );
        assert_eq!(
            first,
            vec![GenChunk::SessionRef {
                id: "ses_abc".into()
            }]
        );
        let second = chat_line_to_chunks(
            r#"{"type":"text","sessionID":"ses_abc","part":{"text":"Hello"}}"#,
            &mut seen,
        );
        // session already reported → text only
        assert_eq!(
            second,
            vec![GenChunk::Text {
                text: "Hello".into()
            }]
        );
    }

    /// 续聊时 `per_turn` 必须随消息带上:opencode 只在建会话那次收到
    /// system,不重申的话「关掉某个开关」对老会话不生效(真机踩坑)。
    #[test]
    fn continued_session_prompt_carries_per_turn_rules() {
        let req = ChatRequest {
            model: "m".into(),
            system: "SYSTEM SETUP".into(),
            turns: vec![crate::types::ChatTurn {
                role: crate::types::ChatRole::User,
                text: "hello there".into(),
            }],
            session: Some("ses_1".into()),
            per_turn: "RULES FOR THIS TURN".into(),
            max_tokens: None,
            temperature: None,
        };
        let prompt = build_chat_prompt(&req);
        assert!(prompt.starts_with("RULES FOR THIS TURN"));
        assert!(prompt.ends_with("hello there"));
        // 续聊不重发 system(上文在服务端,重发只是浪费 token)
        assert!(!prompt.contains("SYSTEM SETUP"));

        // 没有 per_turn 时就是干净的用户消息
        let bare = build_chat_prompt(&ChatRequest {
            per_turn: String::new(),
            ..req.clone()
        });
        assert_eq!(bare, "hello there");
    }

    #[test]
    fn fresh_session_prompt_carries_system() {
        let prompt = build_chat_prompt(&ChatRequest {
            model: "m".into(),
            system: "SYSTEM SETUP".into(),
            turns: vec![crate::types::ChatTurn {
                role: crate::types::ChatRole::User,
                text: "hi".into(),
            }],
            session: None,
            per_turn: "RULES".into(),
            max_tokens: None,
            temperature: None,
        });
        assert!(prompt.starts_with("SYSTEM SETUP"));
        assert!(prompt.ends_with("hi"));
    }

    #[test]
    fn chat_lines_tolerate_plain_text_without_session() {
        let mut seen = false;
        assert_eq!(
            chat_line_to_chunks("plain fallback", &mut seen),
            vec![GenChunk::Text {
                text: "plain fallback\n".into()
            }]
        );
        assert!(!seen);
        assert!(chat_line_to_chunks("   ", &mut seen).is_empty());
    }

    #[test]
    fn sandbox_config_denies_all_tools() {
        let v: serde_json::Value = serde_json::from_str(SANDBOX_OPENCODE_JSON).unwrap();
        let perm = v.pointer("/agent/sf-gen/permission").unwrap();
        for tool in ["edit", "bash", "webfetch"] {
            assert_eq!(perm.get(tool).and_then(|p| p.as_str()), Some("deny"));
        }
    }
}
