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
use crate::types::{ChannelError, ChannelStatus, GenChunk, GenRequest, MeterKind, ModelInfo};
use futures::StreamExt;
use futures::stream::BoxStream;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

/// Probe timeout (§11.C: 全程只读、超时 3s).
const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

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
        if let Some(p) = &self.cfg.bin_override {
            return p.exists().then(|| p.clone());
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

    async fn run_capture(&self, bin: &Path, args: &[&str]) -> Result<String, ChannelError> {
        let fut = Command::new(bin)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output();
        let out = tokio::time::timeout(PROBE_TIMEOUT, fut)
            .await
            .map_err(|_| ChannelError::Timeout)?
            .map_err(|e| ChannelError::Process(e.to_string()))?;
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

#[async_trait::async_trait]
impl ChannelAdapter for OpencodeChannel {
    async fn probe(&self) -> ChannelStatus {
        // ① binary present?
        let Some(bin) = self.find_binary() else {
            return ChannelStatus::NotInstalled;
        };
        // ② version baseline / known-bad (§11.C).
        match self.run_capture(&bin, &["--version"]).await {
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
            Err(e) => {
                return ChannelStatus::Error {
                    message: e.zh_message(),
                };
            }
        }
        // ③ login state via `opencode models` output only (never auth.json).
        match self.run_capture(&bin, &["models"]).await {
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
        let bin = self.find_binary().ok_or(ChannelError::NotInstalled)?;
        std::fs::create_dir_all(&self.cfg.sandbox_dir)
            .map_err(|e| ChannelError::Process(e.to_string()))?;
        std::fs::write(
            self.cfg.sandbox_dir.join("opencode.json"),
            SANDBOX_OPENCODE_JSON,
        )
        .map_err(|e| ChannelError::Process(e.to_string()))?;

        // Standalone run per request — see module docs for why --attach is
        // deliberately not used.
        let mut child = Command::new(&bin)
            .args(["run", "-m", &req.model, "--format", "json"])
            .current_dir(&self.cfg.sandbox_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| ChannelError::Process(e.to_string()))?;

        // Prompt goes through stdin (§3.4): system + user with a separator.
        let prompt = format!("{}\n\n---\n\n{}", req.system, req.user);
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| ChannelError::Process("failed to open opencode stdin".into()))?;
        tokio::spawn(async move {
            let _ = stdin.write_all(prompt.as_bytes()).await;
            let _ = stdin.shutdown().await;
        });

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| ChannelError::Process("failed to open opencode stdout".into()))?;
        let lines = BufReader::new(stdout).lines();

        let stream =
            futures::stream::unfold((lines, Some(child)), |(mut lines, mut child)| async move {
                loop {
                    match lines.next_line().await {
                        Ok(Some(line)) => {
                            if let Some(chunk) = run_line_to_chunk(&line) {
                                return Some((Ok(chunk), (lines, child)));
                            }
                            // non-text event line: keep reading
                        }
                        Ok(None) => {
                            // stdout closed: reap the child and finish.
                            if let Some(c) = child.as_mut() {
                                match c.wait().await {
                                    Ok(status) if !status.success() => {
                                        let err = ChannelError::Process(format!(
                                            "opencode run exited with {status}"
                                        ));
                                        return Some((Err(err), (lines, None)));
                                    }
                                    _ => {}
                                }
                                child = None;
                                return Some((Ok(GenChunk::Done), (lines, child)));
                            }
                            return None;
                        }
                        Err(e) => {
                            return Some((
                                Err(ChannelError::Process(e.to_string())),
                                (lines, None),
                            ));
                        }
                    }
                }
            });
        Ok(stream.boxed())
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

/// Parse `opencode models` output: one model id per line; login state is
/// "has at least one `opencode/` entry" (§11.C).
pub fn parse_models_output(out: &str) -> Vec<ModelInfo> {
    out.lines()
        .map(str::trim)
        .filter(|l| l.starts_with("opencode/"))
        .map(|id| ModelInfo {
            display_name: id.strip_prefix("opencode/").unwrap_or(id).to_string(),
            id: id.to_string(),
            terms_note: crate::channels::zen::FREE_TERMS_NOTE.to_string(),
        })
        .collect()
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
    fn sandbox_config_denies_all_tools() {
        let v: serde_json::Value = serde_json::from_str(SANDBOX_OPENCODE_JSON).unwrap();
        let perm = v.pointer("/agent/sf-gen/permission").unwrap();
        for tool in ["edit", "bash", "webfetch"] {
            assert_eq!(perm.get(tool).and_then(|p| p.as_str()), Some("deny"));
        }
    }
}
