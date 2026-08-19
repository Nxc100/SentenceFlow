//! AI 聊天模块 backend(doc/AI聊天模块-实现方案.md)。
//!
//! 三种模式共用 chat_thread/chat_message 两张表:
//! * **free / roleplay** — 经 [`sf_llm::ChannelAdapter::chat_stream`] 走用户
//!   选定的通道;API 通道回放最近 [`HISTORY_WINDOW`] 条消息,opencode 通道
//!   用 `-s` 服务端会话记忆(sessionID 经 `GenChunk::SessionRef` 回传落库);
//! * **agent** — 直接驱动本机 opencode CLI(`run --format json`,用户选定
//!   的工作目录),工具活动经 `chat://tool` 可视化。实机 spike(2026-08-19)
//!   确认:非交互 run 模式下默认智能体的工具**不经确认直接执行**,因此
//!   安全警告在会话创建时无条件展示,不提供 `--auto`(有意偏离方案 §3.5,
//!   记录于 doc/开发状态.md)。
//!
//! 纠错协议(§3.4):AI 正文末尾附加一行 `⟦fix⟧{json}`;[`split_fix`] 容错
//! 剥离 —— 解析失败时整段按纯文本保留,绝不丢内容。
//!
//! 聊天不写 SRS、不计练习日志、不占试用限额;用量照记 spend(§4.6)。

use crate::channels;
use crate::error::{CmdError, CmdResult};
use crate::progress::{ChatMessageRow, ChatThreadRow};
use crate::state::{AppState, now_unix};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use sf_core::sentence::LevelId;
use sf_llm::ChannelAdapter;
use sf_llm::types::{ChannelError, ChannelId, ChatRequest, ChatRole, ChatTurn, GenChunk};
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, State};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

type S<'a> = State<'a, Arc<AppState>>;

/// API 通道的历史回放窗口(条数;12 轮对话,控 token — §4.2)。
const HISTORY_WINDOW: u32 = 24;
/// 单线程消息上限(§4.2:超出提示开新话题)。
const THREAD_MESSAGE_CAP: u32 = 500;
/// 智能体空转超时:这么久没有任何事件就停掉子进程(权限询问挂起兜底)。
const AGENT_IDLE_TIMEOUT: Duration = Duration::from_secs(180);
/// 聊天空转超时:opencode CLI 真机上出现过续聊无限挂起,不能让界面一直转。
const CHAT_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
/// 纠错协议标记(生僻括号,避免撞正文 — §3.4)。
const FIX_MARKER: &str = "⟦fix⟧";

// ------------------------------------------------------------- ipc payloads

/// 会话列表条目(role_system 不外发,前端不需要)。
#[derive(Debug, Serialize)]
pub struct ChatThreadInfo {
    pub id: i64,
    pub mode: String,
    pub title: String,
    pub role_id: String,
    pub workdir: String,
    /// 本会话固定的通道/模型(空 = 跟随设置)
    pub channel: String,
    pub model: String,
    pub model_label: String,
    pub updated_at: i64,
}

impl From<ChatThreadRow> for ChatThreadInfo {
    fn from(t: ChatThreadRow) -> Self {
        Self {
            id: t.id,
            mode: t.mode,
            title: t.title,
            role_id: t.role_id,
            workdir: t.workdir,
            channel: t.channel,
            model: t.model,
            model_label: t.model_label,
            updated_at: t.updated_at,
        }
    }
}

/// 删除会话的结果(智能体可选连带清理工作目录)。
#[derive(Debug, Serialize)]
pub struct DeleteOutcome {
    /// 工作目录已移入回收站
    pub workdir_trashed: bool,
    /// 给用户看的一句话说明(未清理时说明原因)
    pub note: String,
}

/// 一条历史消息;fix 为空表示无纠错卡。
#[derive(Debug, Serialize)]
pub struct ChatMessage {
    pub id: i64,
    pub role: String,
    pub text: String,
    pub fix: Option<FixCard>,
    /// 智能体这一轮的任务清单(空 = 没有)
    pub todos: Vec<TodoItem>,
    pub ts: i64,
}

impl From<ChatMessageRow> for ChatMessage {
    fn from(m: ChatMessageRow) -> Self {
        let fix = serde_json::from_str(&m.fix_json).ok();
        let todos = serde_json::from_str(&m.todo_json).unwrap_or_default();
        Self {
            id: m.id,
            role: m.role,
            text: m.text,
            fix,
            todos,
            ts: m.ts,
        }
    }
}

/// 纠错小卡(§3.4):更好的说法 + 一句为什么。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FixCard {
    pub better: String,
    pub why: String,
}

#[derive(Debug, Serialize, Clone)]
struct ChunkPayload {
    thread_id: i64,
    text: String,
}

#[derive(Debug, Serialize, Clone)]
struct ToolPayload {
    thread_id: i64,
    label: String,
    /// running | completed | pending | error(照传 opencode 的 state.status)
    status: String,
    /// skill = 技能加载(界面用 🧠 单独呈现),tool = 普通工具
    kind: String,
}

/// 智能体的任务清单快照(opencode `todowrite` 工具;它每推进一步就重发
/// 一次完整清单,界面按最新一份渲染即可)。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TodoItem {
    pub content: String,
    /// pending | in_progress | completed(照传 opencode 的取值)
    pub status: String,
}

#[derive(Debug, Serialize, Clone)]
struct TodoPayload {
    thread_id: i64,
    todos: Vec<TodoItem>,
}

#[derive(Debug, Serialize, Clone)]
struct DonePayload {
    thread_id: i64,
    text: String,
    fix: Option<FixCard>,
    /// true = 被用户停止/超时截断的部分回复
    partial: bool,
}

#[derive(Debug, Serialize, Clone)]
struct ErrorPayload {
    thread_id: i64,
    message: String,
    retry_after_secs: Option<u64>,
}

// ------------------------------------------------------------- prompts

/// 难度自适应描述(英文,面向模型;界面永远不露这些)。
fn level_descriptor(level: LevelId) -> &'static str {
    match level {
        LevelId::L1 => {
            "absolute beginner — use only very common words and very short simple sentences"
        }
        LevelId::L2 => "elementary — everyday vocabulary, short simple sentences",
        LevelId::L3 => "intermediate — everyday conversation, simple compound sentences are fine",
        LevelId::L4 => {
            "upper-intermediate — can follow phone calls and stories; natural pace is fine"
        }
        LevelId::L5 => "advanced — discusses opinions; idiomatic English is fine",
        LevelId::L6 => "proficient — near-native workplace English is fine",
    }
}

/// 纠错协议段(§3.4)。输出契约:正文后独立一行 `⟦fix⟧{json}`。
const FIX_PROTOCOL: &str = r#"After your conversational reply, add one extra final line in exactly this format:
⟦fix⟧{"ok":true}
if the learner's last message was natural, correct English, or:
⟦fix⟧{"ok":false,"better":"<corrected, natural version of their sentence>","why":"<一句简体中文,说明为什么这样更好>"}
if it had mistakes or sounded unnatural. Judge only the learner's most recent message. Never mention this line or the correction inside your conversational reply."#;

/// 自由聊天 system(§3.2:全英文、2–4 句、追问收尾、难度自适应)。
fn build_free_system(level: LevelId, can_do: &str, fix_enabled: bool) -> String {
    let mut s = format!(
        "You are a friendly, patient English conversation partner helping a Chinese learner practice everyday English.\n\
         Learner level: {}.{}\n\
         Rules:\n\
         - Reply in English only, 2-4 short sentences.\n\
         - Match your vocabulary and sentence length to the learner's level so they can read you comfortably.\n\
         - Be warm and encouraging: react to what they said, then end with one simple follow-up question.\n\
         - Do not lecture about grammar inside the conversation.",
        level_descriptor(level),
        can_do_line(can_do),
    );
    push_fix_rule(&mut s, fix_enabled);
    s
}

/// 角色扮演 system(§3.3:保持人设、推进剧情、难度自适应)。
fn build_roleplay_system(
    role_system: &str,
    level: LevelId,
    can_do: &str,
    fix_enabled: bool,
) -> String {
    let mut s = format!(
        "You are role-playing in English with a Chinese learner practicing real-life conversation.\n\
         Your role: {}\n\
         Rules:\n\
         - Stay in character the whole time; never break character or mention being an AI.\n\
         - Reply in English only, 1-4 short sentences, vocabulary matched to the learner's level: {}.{}\n\
         - Keep the scene moving: end most replies with a question or a natural prompt for the learner.\n\
         - If the learner seems stuck, offer a gentle hint in character.",
        role_system.trim(),
        level_descriptor(level),
        can_do_line(can_do),
    );
    push_fix_rule(&mut s, fix_enabled);
    s
}

fn can_do_line(can_do: &str) -> String {
    if can_do.is_empty() {
        String::new()
    } else {
        format!(" Things they can already handle: {can_do}.")
    }
}

fn push_fix_rule(s: &mut String, fix_enabled: bool) {
    s.push('\n');
    if fix_enabled {
        s.push_str(FIX_PROTOCOL);
    } else {
        s.push_str("- Do not correct the learner's English unless they explicitly ask.");
    }
}

// ------------------------------------------------------------- fix parsing

/// 从完整回复剥离纠错标记(§3.4)。返回(正文, 纠错卡)。
/// 容错:标记缺失或 JSON 坏 → 整段原样返回、无卡,绝不丢内容;
/// `ok:true` → 只剥标记行,无卡。
pub fn split_fix(text: &str) -> (String, Option<FixCard>) {
    let Some(idx) = text.rfind(FIX_MARKER) else {
        return (text.trim().to_string(), None);
    };
    let after = &text[idx + FIX_MARKER.len()..];
    // 模型偶尔会在 JSON 后再带空白/杂尾:取首个 '{' 到末个 '}' 之间。
    let json_slice = match (after.find('{'), after.rfind('}')) {
        (Some(a), Some(b)) if a < b => &after[a..=b],
        _ => return (text.trim().to_string(), None),
    };
    #[derive(Deserialize)]
    struct FixWire {
        ok: Option<bool>,
        #[serde(default)]
        better: String,
        #[serde(default)]
        why: String,
    }
    let Ok(wire) = serde_json::from_str::<FixWire>(json_slice) else {
        return (text.trim().to_string(), None);
    };
    let body = text[..idx].trim().to_string();
    let fix = (wire.ok == Some(false) && !wire.better.trim().is_empty()).then(|| FixCard {
        better: wire.better.trim().to_string(),
        why: wire.why.trim().to_string(),
    });
    (body, fix)
}

// ------------------------------------------------------------- thread commands

#[tauri::command]
pub fn chat_thread_create(
    state: S<'_>,
    mode: String,
    title: String,
    role_id: String,
    role_system: String,
    opener: String,
    workdir: String,
) -> CmdResult<ChatThreadInfo> {
    if !matches!(mode.as_str(), "free" | "roleplay" | "agent") {
        return Err(CmdError::new("chat", format!("未知聊天模式: {mode}")));
    }
    if mode == "agent" {
        let dir = Path::new(&workdir);
        if workdir.trim().is_empty() || !dir.is_dir() {
            return Err(CmdError::new(
                "bad_workdir",
                "工作目录不存在——请选择一个真实存在的文件夹",
            ));
        }
    }
    let now = now_unix();
    let title = if title.trim().is_empty() {
        "新对话".to_string()
    } else {
        title.trim().chars().take(40).collect()
    };
    let progress = state.progress.lock().expect("progress lock");
    let id = progress.chat_thread_create(&mode, &title, &role_id, &role_system, &workdir, now)?;
    if !opener.trim().is_empty() {
        // 角色开场白:作为首条 AI 消息落库,也进入后续上下文(§3.3)
        progress.chat_message_add(id, "assistant", opener.trim(), "", now)?;
    }
    let thread = progress
        .chat_thread_get(id)?
        .ok_or_else(|| CmdError::new("chat", "创建会话失败"))?;
    Ok(thread.into())
}

#[tauri::command]
pub fn chat_threads(state: S<'_>) -> CmdResult<Vec<ChatThreadInfo>> {
    let progress = state.progress.lock().expect("progress lock");
    Ok(progress
        .chat_threads()?
        .into_iter()
        .map(Into::into)
        .collect())
}

#[tauri::command]
pub fn chat_history(state: S<'_>, thread_id: i64) -> CmdResult<Vec<ChatMessage>> {
    let progress = state.progress.lock().expect("progress lock");
    Ok(progress
        .chat_messages(thread_id)?
        .into_iter()
        .map(Into::into)
        .collect())
}

/// 本会话固定模型(把 opencode 的 `/model` 包成可视化操作):
/// `channel` 为空 = 恢复跟随全局设置。下一条消息起生效;opencode 会话
/// 换模型后仍沿用同一 `-s` 会话(实机验证记忆不丢)。
#[tauri::command]
pub fn chat_thread_set_model(
    state: S<'_>,
    thread_id: i64,
    channel: Option<ChannelId>,
    model: String,
    model_label: String,
) -> CmdResult<ChatThreadInfo> {
    let progress = state.progress.lock().expect("progress lock");
    let ch = channel.map(channel_key).unwrap_or_default();
    // 通道为空即"跟随设置",模型一并清掉,避免半吊子状态
    let (model, label) = if ch.is_empty() {
        (String::new(), String::new())
    } else {
        (model, model_label)
    };
    progress.chat_thread_set_model(thread_id, ch, &model, &label)?;
    let thread = progress
        .chat_thread_get(thread_id)?
        .ok_or_else(|| CmdError::new("chat", "会话不存在"))?;
    Ok(thread.into())
}

#[tauri::command]
pub fn chat_thread_delete(
    state: S<'_>,
    thread_id: i64,
    delete_workdir: bool,
) -> CmdResult<DeleteOutcome> {
    // 先停掉这个会话可能还在跑的流(不能一边删一边写)
    state.chat_cancel(thread_id);
    let workdir = {
        let progress = state.progress.lock().expect("progress lock");
        let thread = progress.chat_thread_get(thread_id)?;
        progress.chat_thread_delete(thread_id)?;
        thread.filter(|t| t.mode == "agent").map(|t| t.workdir)
    };
    if !delete_workdir {
        return Ok(DeleteOutcome {
            workdir_trashed: false,
            note: String::new(),
        });
    }
    let Some(dir) = workdir.filter(|d| !d.trim().is_empty()) else {
        return Ok(DeleteOutcome {
            workdir_trashed: false,
            note: "这个会话没有关联文件夹".into(),
        });
    };
    match trash_workdir(&state, Path::new(&dir)) {
        Ok(()) => Ok(DeleteOutcome {
            workdir_trashed: true,
            note: format!("文件夹已移入回收站:{dir}"),
        }),
        Err(why) => Ok(DeleteOutcome {
            workdir_trashed: false,
            note: format!("会话已删除,但文件夹保留了:{why}"),
        }),
    }
}

/// 把智能体工作目录移入系统回收站(可从回收站找回,比直接抹掉安全)。
/// 层层设防:只删真实存在的普通目录,系统目录/用户主目录/桌面文档等
/// 一律拒绝,应用自身数据目录及其祖先也拒绝。
fn trash_workdir(state: &AppState, dir: &Path) -> Result<(), String> {
    let dir = dir
        .canonicalize()
        .map_err(|_| "文件夹不存在或已被移走".to_string())?;
    if !dir.is_dir() {
        return Err("这不是一个文件夹".into());
    }
    if dir.parent().is_none() {
        return Err("这是磁盘根目录,不能删".into());
    }
    for guard in protected_dirs(state) {
        let Ok(guard) = guard.canonicalize() else {
            continue;
        };
        if dir == guard {
            return Err("这是系统或个人重要目录,不能删".into());
        }
        // 目标是受保护目录的祖先 ⇒ 删它会连带删掉受保护目录
        if guard.starts_with(&dir) {
            return Err("这个文件夹里包含系统或应用数据,不能删".into());
        }
    }
    trash::delete(&dir).map_err(|e| format!("系统拒绝了这次删除({e})"))
}

/// 绝不允许被清理的目录清单。
fn protected_dirs(state: &AppState) -> Vec<std::path::PathBuf> {
    let mut dirs = vec![state.paths.root.clone()];
    if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
        let home = std::path::PathBuf::from(home);
        for sub in [
            "",
            "Desktop",
            "Documents",
            "Downloads",
            "Pictures",
            "Music",
            "Videos",
            "OneDrive",
            "桌面",
            "文档",
            "下载",
        ] {
            dirs.push(if sub.is_empty() {
                home.clone()
            } else {
                home.join(sub)
            });
        }
    }
    for var in [
        "SystemRoot",
        "ProgramFiles",
        "ProgramFiles(x86)",
        "ProgramData",
    ] {
        if let Some(v) = std::env::var_os(var) {
            dirs.push(std::path::PathBuf::from(v));
        }
    }
    if let Some(users) = std::env::var_os("SystemDrive") {
        let drive = std::path::PathBuf::from(format!("{}\\", users.to_string_lossy()));
        dirs.push(drive.join("Users"));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        dirs.push(parent.to_path_buf());
    }
    dirs
}

/// [停止]:只叫停这一个会话的流(其余会话继续),保留已收到的部分。
#[tauri::command]
pub fn chat_stop(state: S<'_>, thread_id: i64) -> CmdResult<()> {
    state.chat_cancel(thread_id);
    Ok(())
}

/// 仍在生成回复的会话 id —— 前端离开页面再回来时据此恢复「生成中」指示
/// (回复本身由随后的 `chat://done` 整段补齐)。
#[tauri::command]
pub fn chat_active_threads(state: S<'_>) -> CmdResult<Vec<i64>> {
    Ok(state
        .chat_cancels
        .lock()
        .expect("chat cancels lock")
        .keys()
        .copied()
        .collect())
}

/// 系统文件夹选择器(智能体工作目录)。返回 None = 用户取消。
#[tauri::command]
pub async fn pick_folder(title: String) -> CmdResult<Option<String>> {
    let picked = rfd::AsyncFileDialog::new()
        .set_title(&title)
        .pick_folder()
        .await;
    Ok(picked.map(|h| h.path().to_string_lossy().into_owned()))
}

// ------------------------------------------------------------- model resolution

pub fn channel_key(c: ChannelId) -> &'static str {
    match c {
        ChannelId::Opencode => "opencode",
        ChannelId::Deepseek => "deepseek",
        ChannelId::Zen => "zen",
        ChannelId::Ollama => "ollama",
    }
}

pub fn channel_from_key(s: &str) -> Option<ChannelId> {
    match s {
        "opencode" => Some(ChannelId::Opencode),
        "deepseek" => Some(ChannelId::Deepseek),
        "zen" => Some(ChannelId::Zen),
        "ollama" => Some(ChannelId::Ollama),
        _ => None,
    }
}

/// 本会话固定的通道/模型优先(每会话切模型);会话没定或定的通道已被
/// 内容包策略停用 → 回落全局设置。
fn resolve_channel_model(
    state: &AppState,
    thread: &ChatThreadRow,
) -> CmdResult<(ChannelId, String)> {
    if let Some(ch) = channel_from_key(&thread.channel)
        && state.policy.is_enabled(ch)
        && !thread.model.is_empty()
    {
        return Ok((ch, thread.model.clone()));
    }
    let settings = state.settings.lock().expect("settings lock");
    let ch = settings
        .ai
        .channel
        .ok_or_else(|| CmdError::new("no_channel", "未配置 AI 通道"))?;
    Ok((ch, settings.ai.model.clone().unwrap_or_default()))
}

// ------------------------------------------------------------- chat send

#[tauri::command]
pub async fn chat_send(
    app: AppHandle,
    state: S<'_>,
    thread_id: i64,
    text: String,
    fix_enabled: bool,
) -> CmdResult<()> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err(CmdError::new("chat", "消息为空"));
    }
    let level = {
        let settings = state.settings.lock().expect("settings lock");
        settings.level.unwrap_or(LevelId::L1)
    };
    let (thread, turns) = {
        let progress = state.progress.lock().expect("progress lock");
        let thread = progress
            .chat_thread_get(thread_id)?
            .ok_or_else(|| CmdError::new("chat", "会话不存在"))?;
        if thread.mode == "agent" {
            return Err(CmdError::new("chat", "智能体会话请走 agent_send"));
        }
        if progress.chat_message_count(thread_id)? >= THREAD_MESSAGE_CAP {
            return Err(CmdError::new(
                "thread_full",
                "这个对话有点长了——开个新话题继续吧(这里的记录会保留)",
            ));
        }
        let now = now_unix();
        let mut turns: Vec<ChatTurn> = progress
            .chat_recent_messages(thread_id, HISTORY_WINDOW)?
            .into_iter()
            .map(|m| ChatTurn {
                role: if m.role == "user" {
                    ChatRole::User
                } else {
                    ChatRole::Assistant
                },
                text: m.text,
            })
            .collect();
        turns.push(ChatTurn {
            role: ChatRole::User,
            text: text.clone(),
        });
        progress.chat_message_add(thread_id, "user", &text, "", now)?;
        progress.chat_thread_touch(thread_id, now)?;
        (thread, turns)
    };
    // 本会话固定的通道/模型优先,否则跟随全局设置(每会话切模型)
    let (channel, model) = resolve_channel_model(&state, &thread)?;
    let can_do = state
        .spec_for(level)
        .map(|s| s.can_do.join("、"))
        .unwrap_or_default();
    let system = match thread.mode.as_str() {
        "roleplay" => build_roleplay_system(&thread.role_system, level, &can_do, fix_enabled),
        _ => build_free_system(level, &can_do, fix_enabled),
    };
    // opencode 服务端会话只在仍用 opencode 通道时续用(换通道后回放本地历史)
    let session = (channel == ChannelId::Opencode && !thread.oc_session.is_empty())
        .then(|| thread.oc_session.clone());
    let req = ChatRequest {
        model,
        system,
        turns,
        session,
        max_tokens: Some(1024),
        temperature: Some(0.7),
    };
    let adapter = channels::make_adapter(&state, channel, None)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn(async move {
        run_chat_stream(app, state, thread_id, adapter, req).await;
    });
    Ok(())
}

/// 一轮流式的结局。
enum StreamOutcome {
    /// 正常收流(或被用户停止),带已收到的正文
    Finished { text: String, cancelled: bool },
    /// 传输层失败,带已收到的正文
    Failed { text: String, error: ChannelError },
}

async fn run_chat_stream(
    app: AppHandle,
    state: Arc<AppState>,
    thread_id: i64,
    adapter: Box<dyn ChannelAdapter>,
    req: ChatRequest,
) {
    // 本会话专属停止信号:切到别的会话不影响这个流,[停止] 也只掐这一个
    let cancel_handle = state.chat_cancel_register(thread_id);
    // 续聊拿到空回复时的重开方案:丢掉服务端会话、用本地历史从头说一遍。
    // 本地 chat_message 才是事实源,opencode 会话只是省 token 的优化。
    let retry = req.session.is_some().then(|| ChatRequest {
        session: None,
        ..req.clone()
    });

    let mut outcome = stream_once(
        &app,
        &state,
        thread_id,
        adapter.as_ref(),
        req,
        &cancel_handle,
    )
    .await;

    if let StreamOutcome::Finished {
        text,
        cancelled: false,
    } = &outcome
        && text.trim().is_empty()
        && let Some(retry_req) = retry
    {
        // 实机遇到过:某个 opencode 会话续聊后只回 step_finish、不吐 text
        // (tokens 照扣)。会话废了就换一个,用户无感。
        {
            let progress = state.progress.lock().expect("progress lock");
            let _ = progress.chat_thread_set_session(thread_id, "");
        }
        outcome = stream_once(
            &app,
            &state,
            thread_id,
            adapter.as_ref(),
            retry_req,
            &cancel_handle,
        )
        .await;
    }

    state.chat_cancel_finish(thread_id, &cancel_handle);
    match outcome {
        StreamOutcome::Finished { text, cancelled } => {
            finalize_reply(&app, &state, thread_id, &text, cancelled);
            if text.trim().is_empty() && !cancelled {
                // 绝不留白:没有回复也要明说,别让用户以为卡住了
                let _ = app.emit(
                    "chat://error",
                    ErrorPayload {
                        thread_id,
                        message: "这次没有收到回复(可能被限速,或这轮上下文太长)——\
                                  可以再发一次,或在上方换个模型试试"
                            .into(),
                        retry_after_secs: None,
                    },
                );
            }
        }
        StreamOutcome::Failed { text, error } => {
            // 出错也不丢已收内容:先落部分回复再报错
            finalize_reply(&app, &state, thread_id, &text, true);
            emit_chat_error(&app, thread_id, &error);
        }
    }
}

/// 跑一轮流:把 text 块实时推给前端,返回累计正文。
async fn stream_once(
    app: &AppHandle,
    state: &Arc<AppState>,
    thread_id: i64,
    adapter: &dyn ChannelAdapter,
    req: ChatRequest,
    cancel_handle: &tokio::sync::Notify,
) -> StreamOutcome {
    let mut stream = match adapter.chat_stream(req).await {
        Ok(s) => s,
        Err(error) => {
            return StreamOutcome::Failed {
                text: String::new(),
                error,
            };
        }
    };
    let mut full = String::new();
    let mut cancel = Box::pin(cancel_handle.notified());
    loop {
        tokio::select! {
            _ = &mut cancel => {
                return StreamOutcome::Finished { text: full, cancelled: true };
            }
            // 彻底没动静的兜底(CLI 真机上出现过无限挂起)
            _ = tokio::time::sleep(CHAT_IDLE_TIMEOUT) => {
                return StreamOutcome::Failed {
                    text: full,
                    error: ChannelError::Timeout,
                };
            }
            chunk = stream.next() => match chunk {
                Some(Ok(GenChunk::Text { text })) => {
                    full.push_str(&text);
                    let _ = app.emit("chat://chunk", ChunkPayload { thread_id, text });
                }
                Some(Ok(GenChunk::SessionRef { id })) => {
                    let progress = state.progress.lock().expect("progress lock");
                    let _ = progress.chat_thread_set_session(thread_id, &id);
                }
                Some(Ok(GenChunk::Usage { prompt_tokens, completion_tokens })) => {
                    let progress = state.progress.lock().expect("progress lock");
                    let _ = progress.spend_add(now_unix(), "chat", prompt_tokens, completion_tokens, 0.0);
                }
                Some(Ok(GenChunk::Done)) | None => {
                    return StreamOutcome::Finished { text: full, cancelled: false };
                }
                Some(Err(error)) => return StreamOutcome::Failed { text: full, error },
            }
        }
    }
}

/// 剥纠错标记 → 落库 → 发 done。partial = 被停止/出错截断。
fn finalize_reply(
    app: &AppHandle,
    state: &Arc<AppState>,
    thread_id: i64,
    full: &str,
    partial: bool,
) {
    let (body, fix) = split_fix(full);
    if !body.is_empty() {
        let fix_json = fix
            .as_ref()
            .and_then(|f| serde_json::to_string(f).ok())
            .unwrap_or_default();
        let progress = state.progress.lock().expect("progress lock");
        let now = now_unix();
        let _ = progress.chat_message_add(thread_id, "assistant", &body, &fix_json, now);
        let _ = progress.chat_thread_touch(thread_id, now);
    }
    let _ = app.emit(
        "chat://done",
        DonePayload {
            thread_id,
            text: body,
            fix,
            partial,
        },
    );
}

fn emit_chat_error(app: &AppHandle, thread_id: i64, e: &ChannelError) {
    let retry_after_secs = match e {
        ChannelError::RateLimited { retry_after_secs } => Some(*retry_after_secs),
        _ => None,
    };
    let _ = app.emit(
        "chat://error",
        ErrorPayload {
            thread_id,
            message: e.zh_message(),
            retry_after_secs,
        },
    );
}

// ------------------------------------------------------------- agent mode

/// 智能体 stdout 一行解析出的事件(纯函数可测)。
#[derive(Debug, PartialEq)]
pub enum AgentLineEvent {
    Text(String),
    Tool {
        label: String,
        status: String,
        /// skill = 加载技能,tool = 普通工具
        kind: String,
    },
    Session(String),
    Tokens {
        input: u64,
        output: u64,
    },
    /// 任务清单快照(todowrite 的完整列表)
    Todos(Vec<TodoItem>),
}

/// 解析一行 `opencode run --format json` 事件(形状为 2026-08-19 实机
/// spike 钉死:tool_use 带 part.tool/part.state.{status,title},
/// step_finish 带 part.tokens)。非 JSON 行按纯文本透传。
pub fn parse_agent_line(line: &str) -> Vec<AgentLineEvent> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) else {
        return vec![AgentLineEvent::Text(format!("{trimmed}\n"))];
    };
    let mut out = Vec::new();
    if let Some(id) = v.get("sessionID").and_then(|s| s.as_str())
        && !id.is_empty()
    {
        out.push(AgentLineEvent::Session(id.to_string()));
    }
    match v.get("type").and_then(|t| t.as_str()) {
        Some("text") => {
            if let Some(t) = v.pointer("/part/text").and_then(|t| t.as_str())
                && !t.is_empty()
            {
                out.push(AgentLineEvent::Text(t.to_string()));
            }
        }
        Some("tool_use") => {
            let tool = v
                .pointer("/part/tool")
                .and_then(|t| t.as_str())
                .unwrap_or("tool");
            let title = v
                .pointer("/part/state/title")
                .and_then(|t| t.as_str())
                .unwrap_or("");
            let status = v
                .pointer("/part/state/status")
                .and_then(|t| t.as_str())
                .unwrap_or("running");
            // 任务清单:整份列表回传,界面渲染成勾选清单(与 TUI 的 Todo 面板
            // 同源)。实机事件:tool="todowrite",state.input.todos[]。
            if tool == "todowrite"
                && let Some(items) = v
                    .pointer("/part/state/input/todos")
                    .and_then(|t| t.as_array())
            {
                let todos: Vec<TodoItem> = items
                    .iter()
                    .filter_map(|it| {
                        let content = it.get("content")?.as_str()?.trim().to_string();
                        (!content.is_empty()).then(|| TodoItem {
                            content,
                            status: it
                                .get("status")
                                .and_then(|s| s.as_str())
                                .unwrap_or("pending")
                                .to_string(),
                        })
                    })
                    .collect();
                if !todos.is_empty() {
                    out.push(AgentLineEvent::Todos(todos));
                    // 清单自己就是最好的进度显示,不再多一行「⚙ todowrite」
                    return out;
                }
            }
            // 技能加载单独成一类:界面用「🧠 技能:名字」而不是英文工具名
            // (实机事件:tool="skill",state.input.name / state.metadata.name)
            let skill_name = v
                .pointer("/part/state/input/name")
                .or_else(|| v.pointer("/part/state/metadata/name"))
                .and_then(|t| t.as_str())
                .unwrap_or("");
            let (label, kind) = if tool == "skill" && !skill_name.is_empty() {
                (format!("技能:{skill_name}"), "skill")
            } else if title.is_empty() {
                (tool.to_string(), "tool")
            } else {
                (format!("{tool} · {title}"), "tool")
            };
            out.push(AgentLineEvent::Tool {
                label,
                status: status.to_string(),
                kind: kind.to_string(),
            });
        }
        Some("step_finish") => {
            let input = v
                .pointer("/part/tokens/input")
                .and_then(|t| t.as_u64())
                .unwrap_or(0);
            let output = v
                .pointer("/part/tokens/output")
                .and_then(|t| t.as_u64())
                .unwrap_or(0);
            if input + output > 0 {
                out.push(AgentLineEvent::Tokens { input, output });
            }
        }
        _ => {}
    }
    out
}

/// `skill_path` 非空 = 手动触发型技能:把技能正文注入本轮消息
/// (opencode 不会自动调用这类技能 —— 见 skills.rs 模块说明)。
#[tauri::command]
pub async fn agent_send(
    app: AppHandle,
    state: S<'_>,
    thread_id: i64,
    text: String,
    skill_path: String,
) -> CmdResult<()> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err(CmdError::new("chat", "消息为空"));
    }
    // 界面上显示的仍是用户原话,送给模型的是"技能正文 + 原话"
    let prompt = if skill_path.trim().is_empty() {
        text.clone()
    } else {
        let raw = std::fs::read_to_string(&skill_path)
            .map_err(|e| CmdError::new("skill", format!("读不到这个技能文件:{e}")))?;
        let (fm, body) = crate::skills::parse_skill_md(&raw);
        let name = if fm.name.is_empty() {
            "skill"
        } else {
            &fm.name
        };
        crate::skills::compose_skill_prompt(name, &body, &text)
    };
    let (bin_override, proxy, global_model) = {
        let settings = state.settings.lock().expect("settings lock");
        (
            settings.ai.opencode_bin.clone(),
            settings
                .ai
                .proxy_url
                .clone()
                .filter(|p| !p.trim().is_empty()),
            settings.ai.model.clone(),
        )
    };
    let bin = sf_llm::channels::opencode::locate_binary(bin_override.as_deref().map(Path::new))
        .ok_or_else(|| {
            CmdError::new(
                "not_installed",
                "未找到 opencode——先到「设置 · AI 接入」一键安装",
            )
        })?;
    let thread = {
        let progress = state.progress.lock().expect("progress lock");
        let thread = progress
            .chat_thread_get(thread_id)?
            .ok_or_else(|| CmdError::new("chat", "会话不存在"))?;
        if thread.mode != "agent" {
            return Err(CmdError::new("chat", "该会话不是智能体会话"));
        }
        if progress.chat_message_count(thread_id)? >= THREAD_MESSAGE_CAP {
            return Err(CmdError::new(
                "thread_full",
                "这个会话有点长了——新建一个继续吧(这里的记录会保留)",
            ));
        }
        let now = now_unix();
        progress.chat_message_add(thread_id, "user", &text, "", now)?;
        progress.chat_thread_touch(thread_id, now)?;
        thread
    };
    if !Path::new(&thread.workdir).is_dir() {
        return Err(CmdError::new(
            "bad_workdir",
            "工作目录不存在或已被移动——请新建会话重新选择",
        ));
    }
    // 模型:本会话固定的优先,否则全局设置;两者都不是 opencode 目录下的
    // 名字就交给 CLI 用它自己的默认(智能体跑的就是 opencode)。
    let model = [thread.model.clone(), global_model.unwrap_or_default()]
        .into_iter()
        .find(|m| m.starts_with("opencode/"));
    let state = state.inner().clone();
    tauri::async_runtime::spawn(async move {
        run_agent(app, state, thread, bin, proxy, model, prompt).await;
    });
    Ok(())
}

/// `prompt` 是送给 CLI 的完整提示:可能已注入技能正文,与界面显示的用户
/// 原话不同(手动触发型技能,见 skills.rs)。
async fn run_agent(
    app: AppHandle,
    state: Arc<AppState>,
    thread: ChatThreadRow,
    bin: std::path::PathBuf,
    proxy: Option<String>,
    model: Option<String>,
    prompt: String,
) {
    let thread_id = thread.id;
    // 本会话专属停止信号(切走不打断,[停止] 只掐这一个)
    let cancel_handle = state.chat_cancel_register(thread_id);
    let mut cmd = sf_llm::channels::opencode::hidden_command(&bin, proxy.as_deref());
    cmd.args(["run", "--format", "json"]);
    if let Some(m) = &model {
        cmd.args(["-m", m]);
    }
    if !thread.oc_session.is_empty() {
        cmd.args(["-s", &thread.oc_session]);
    }
    let mut child = match cmd
        .current_dir(&thread.workdir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            state.chat_cancel_finish(thread_id, &cancel_handle);
            emit_chat_error(&app, thread_id, &ChannelError::Process(e.to_string()));
            return;
        }
    };
    if let Some(mut stdin) = child.stdin.take() {
        tokio::spawn(async move {
            let _ = stdin.write_all(prompt.as_bytes()).await;
            let _ = stdin.shutdown().await;
        });
    }
    // stderr 收集:失败时给用户看结尾片段
    let stderr_task = child.stderr.take().map(|mut err| {
        tokio::spawn(async move {
            let mut buf = String::new();
            let _ = err.read_to_string(&mut buf).await;
            buf
        })
    });
    let Some(stdout) = child.stdout.take() else {
        state.chat_cancel_finish(thread_id, &cancel_handle);
        emit_chat_error(
            &app,
            thread_id,
            &ChannelError::Process("failed to open opencode stdout".into()),
        );
        return;
    };
    let mut lines = BufReader::new(stdout).lines();

    enum EndReason {
        Eof,
        Cancelled,
        Idle,
        ReadErr(String),
    }
    let mut full = String::new();
    let mut session_saved = !thread.oc_session.is_empty();
    let mut tokens = (0u64, 0u64);
    // 最新一份任务清单:随消息落库,回头翻会话还能看到这轮干了哪几步
    let mut todos: Vec<TodoItem> = Vec::new();
    let mut cancel = Box::pin(cancel_handle.notified());
    let end = loop {
        tokio::select! {
            _ = &mut cancel => break EndReason::Cancelled,
            // 每收到一行事件重新计时:只拦「彻底没动静」(权限挂起/断网)
            _ = tokio::time::sleep(AGENT_IDLE_TIMEOUT) => break EndReason::Idle,
            line = lines.next_line() => match line {
                Ok(Some(l)) => {
                    for ev in parse_agent_line(&l) {
                        match ev {
                            AgentLineEvent::Text(t) => {
                                full.push_str(&t);
                                let _ = app.emit("chat://chunk", ChunkPayload { thread_id, text: t });
                            }
                            AgentLineEvent::Tool { label, status, kind } => {
                                let _ = app.emit("chat://tool", ToolPayload { thread_id, label, status, kind });
                            }
                            AgentLineEvent::Session(id) => {
                                if !session_saved {
                                    session_saved = true;
                                    let progress = state.progress.lock().expect("progress lock");
                                    let _ = progress.chat_thread_set_session(thread_id, &id);
                                }
                            }
                            AgentLineEvent::Tokens { input, output } => {
                                tokens.0 += input;
                                tokens.1 += output;
                            }
                            AgentLineEvent::Todos(items) => {
                                todos = items.clone();
                                let _ = app.emit("chat://todo", TodoPayload { thread_id, todos: items });
                            }
                        }
                    }
                }
                Ok(None) => break EndReason::Eof,
                Err(e) => break EndReason::ReadErr(e.to_string()),
            }
        }
    };
    state.chat_cancel_finish(thread_id, &cancel_handle);
    if tokens.0 + tokens.1 > 0 {
        let progress = state.progress.lock().expect("progress lock");
        let _ = progress.spend_add(now_unix(), "agent", tokens.0, tokens.1, 0.0);
    }
    match end {
        EndReason::Eof => {
            let failed = matches!(child.wait().await, Ok(status) if !status.success());
            if failed && full.trim().is_empty() {
                let tail = match stderr_task {
                    Some(t) => t.await.unwrap_or_default(),
                    None => String::new(),
                };
                let tail = tail.trim();
                let brief: String = tail
                    .chars()
                    .rev()
                    .take(200)
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                // stderr 尾巴对排障最有价值,原样给用户(不套 zh_message 通配)
                let _ = app.emit(
                    "chat://error",
                    ErrorPayload {
                        thread_id,
                        message: if brief.is_empty() {
                            "opencode 运行失败,可重试或换个模型".into()
                        } else {
                            format!("opencode 运行失败:{brief}")
                        },
                        retry_after_secs: None,
                    },
                );
            } else {
                let empty = full.trim().is_empty();
                finalize_agent_reply(&app, &state, thread_id, &full, false, &todos);
                if empty {
                    // 绝不留白:跑完却没有任何输出时明确告诉用户
                    let _ = app.emit(
                        "chat://error",
                        ErrorPayload {
                            thread_id,
                            message: "这次没有返回内容(可能被限速,或这个会话上下文太长)——\
                                      可以再说一次,或新建一个会话"
                                .into(),
                            retry_after_secs: None,
                        },
                    );
                }
            }
        }
        EndReason::Cancelled => {
            let _ = child.start_kill();
            finalize_agent_reply(&app, &state, thread_id, &full, true, &todos);
        }
        EndReason::Idle => {
            let _ = child.start_kill();
            finalize_agent_reply(&app, &state, thread_id, &full, true, &todos);
            let _ = app.emit(
                "chat://error",
                ErrorPayload {
                    thread_id,
                    message: "AI 长时间没有动静,已停止——可以重试,或换个说法".into(),
                    retry_after_secs: None,
                },
            );
        }
        EndReason::ReadErr(e) => {
            let _ = child.start_kill();
            finalize_agent_reply(&app, &state, thread_id, &full, true, &todos);
            emit_chat_error(&app, thread_id, &ChannelError::Process(e));
        }
    }
}

/// 智能体回复落库 + done(无纠错协议,原文即正文;任务清单随消息存下来)。
fn finalize_agent_reply(
    app: &AppHandle,
    state: &Arc<AppState>,
    thread_id: i64,
    full: &str,
    partial: bool,
    todos: &[TodoItem],
) {
    let body = full.trim().to_string();
    if !body.is_empty() {
        let todo_json = if todos.is_empty() {
            String::new()
        } else {
            serde_json::to_string(todos).unwrap_or_default()
        };
        let progress = state.progress.lock().expect("progress lock");
        let now = now_unix();
        let _ = progress.chat_message_add_full(thread_id, "assistant", &body, "", &todo_json, now);
        let _ = progress.chat_thread_touch(thread_id, now);
    }
    let _ = app.emit(
        "chat://done",
        DonePayload {
            thread_id,
            text: body,
            fix: None,
            partial,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fix_ok_true_strips_marker_without_card() {
        let (body, fix) = split_fix("Nice! What did you eat?\n⟦fix⟧{\"ok\":true}");
        assert_eq!(body, "Nice! What did you eat?");
        assert!(fix.is_none());
    }

    #[test]
    fn fix_ok_false_yields_card() {
        let raw = "Sounds fun! Where did you go?\n⟦fix⟧{\"ok\":false,\"better\":\"I went to the park yesterday.\",\"why\":\"过去的事用过去式 went\"}";
        let (body, fix) = split_fix(raw);
        assert_eq!(body, "Sounds fun! Where did you go?");
        let fix = fix.unwrap();
        assert_eq!(fix.better, "I went to the park yesterday.");
        assert!(fix.why.contains("过去式"));
    }

    #[test]
    fn fix_bad_json_keeps_full_text() {
        let raw = "Hello!\n⟦fix⟧{broken json";
        let (body, fix) = split_fix(raw);
        assert_eq!(body, raw);
        assert!(fix.is_none());
    }

    #[test]
    fn fix_missing_marker_passthrough() {
        let (body, fix) = split_fix("  Just a plain reply.  ");
        assert_eq!(body, "Just a plain reply.");
        assert!(fix.is_none());
    }

    #[test]
    fn fix_tolerates_trailing_junk_after_json() {
        let raw = "Great!\n⟦fix⟧ {\"ok\":false,\"better\":\"He goes to school.\",\"why\":\"三单加 s\"} \n";
        let (body, fix) = split_fix(raw);
        assert_eq!(body, "Great!");
        assert_eq!(fix.unwrap().better, "He goes to school.");
    }

    #[test]
    fn free_system_toggles_fix_protocol() {
        let with = build_free_system(LevelId::L1, "打招呼、自我介绍", true);
        assert!(with.contains("⟦fix⟧"));
        assert!(with.contains("absolute beginner"));
        assert!(with.contains("打招呼、自我介绍"));
        let without = build_free_system(LevelId::L3, "", false);
        assert!(!without.contains("⟦fix⟧"));
        assert!(without.contains("Do not correct"));
    }

    #[test]
    fn roleplay_system_embeds_role() {
        let s = build_roleplay_system(
            "You are a job interviewer at a tech company.",
            LevelId::L4,
            "电话沟通",
            true,
        );
        assert!(s.contains("job interviewer"));
        assert!(s.contains("Stay in character"));
        assert!(s.contains("⟦fix⟧"));
    }

    #[test]
    fn channel_keys_roundtrip() {
        for c in ChannelId::ALL {
            assert_eq!(channel_from_key(channel_key(c)), Some(c));
        }
        assert_eq!(channel_from_key(""), None);
        assert_eq!(channel_from_key("nope"), None);
    }

    #[test]
    fn agent_lines_parse_spike_shapes() {
        // 实机 spike 采样(截断字段)
        let tool = r#"{"type":"tool_use","sessionID":"ses_1","part":{"type":"tool","tool":"bash","state":{"status":"completed","title":"ls -1A"}}}"#;
        let evs = parse_agent_line(tool);
        assert!(evs.contains(&AgentLineEvent::Session("ses_1".into())));
        assert!(evs.contains(&AgentLineEvent::Tool {
            label: "bash · ls -1A".into(),
            status: "completed".into(),
            kind: "tool".into()
        }));

        let text =
            r#"{"type":"text","sessionID":"ses_1","part":{"type":"text","text":"4 files."}}"#;
        assert!(parse_agent_line(text).contains(&AgentLineEvent::Text("4 files.".into())));

        let finish = r#"{"type":"step_finish","sessionID":"ses_1","part":{"type":"step-finish","tokens":{"total":11426,"input":9692,"output":48}}}"#;
        assert!(parse_agent_line(finish).contains(&AgentLineEvent::Tokens {
            input: 9692,
            output: 48
        }));

        assert_eq!(
            parse_agent_line("plain"),
            vec![AgentLineEvent::Text("plain\n".into())]
        );
        assert!(parse_agent_line("   ").is_empty());
    }

    /// 任务清单事件(实机形状:tool="todowrite",state.input.todos[];
    /// 每推进一步重发一次完整清单)。清单自己就是进度显示,不再多一行工具活动。
    #[test]
    fn todowrite_event_yields_full_checklist() {
        let line = r#"{"type":"tool_use","sessionID":"ses_1","part":{"type":"tool","tool":"todowrite","state":{"status":"completed","title":"2 todos","input":{"todos":[{"content":"创建 a.txt","priority":"high","status":"completed"},{"content":"创建 b.txt","priority":"high","status":"in_progress"},{"content":"","status":"pending"}]}}}}"#;
        let evs = parse_agent_line(line);
        assert!(
            evs.iter()
                .all(|e| !matches!(e, AgentLineEvent::Tool { .. }))
        );
        let todos = evs
            .iter()
            .find_map(|e| match e {
                AgentLineEvent::Todos(t) => Some(t),
                _ => None,
            })
            .expect("应解析出任务清单");
        // 空 content 的条目丢掉,其余保序
        assert_eq!(todos.len(), 2);
        assert_eq!(todos[0].content, "创建 a.txt");
        assert_eq!(todos[0].status, "completed");
        assert_eq!(todos[1].status, "in_progress");
    }

    #[test]
    fn todowrite_without_items_falls_back_to_tool_line() {
        let line = r#"{"type":"tool_use","part":{"tool":"todowrite","state":{"status":"running","title":"0 todos","input":{"todos":[]}}}}"#;
        assert!(parse_agent_line(line).iter().any(|e| matches!(
            e,
            AgentLineEvent::Tool { kind, .. } if kind == "tool"
        )));
    }

    /// 技能加载事件单独成一类(实机形状:tool="skill",state.input.name,
    /// title="Loaded skill: x",metadata.dir 指向技能目录)。
    #[test]
    fn skill_tool_event_is_labelled_in_chinese() {
        let line = r#"{"type":"tool_use","sessionID":"ses_1","part":{"type":"tool","tool":"skill","state":{"status":"completed","title":"Loaded skill: tea-brewing","input":{"name":"tea-brewing"},"metadata":{"name":"tea-brewing","dir":"D:\\x\\.opencode\\skills\\tea-brewing"}}}}"#;
        assert!(parse_agent_line(line).contains(&AgentLineEvent::Tool {
            label: "技能:tea-brewing".into(),
            status: "completed".into(),
            kind: "skill".into()
        }));
    }
}
