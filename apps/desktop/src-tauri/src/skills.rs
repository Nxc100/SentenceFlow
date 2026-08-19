//! opencode Agent Skills 的图形化外壳(智能体模式)。
//!
//! **数据源是 opencode 自己的 `debug skill` 命令** —— 去重、根目录优先级、
//! 内置技能全按它的规则来,我们不另写一套(真机验证 2026-08-19:同一台机器
//! 三处全局根目录有大量重名技能,CLI 解析出 18 个,与 TUI 技能面板一致;
//! 其中 `chatgpt-image-beautify` 的目录名是 `2` —— 技能名取自 frontmatter,
//! 不是目录名,自己实现必踩这种坑)。
//!
//! 磁盘上有、却不在该清单里的技能由我们补扫,标为**未加载**并尽量说清原因。
//! 最常见的原因是 frontmatter YAML 不合法 —— 实测 `argument-hint: [a] [b]`
//! (流序列后面又跟一个 token)会让 opencode **整份技能拒收**,而 Claude Code
//! 容忍这种写法,所以从 Claude 那边装过来的技能很容易悄悄失效。
//! 未加载的技能仍可用:把正文直接注入这一轮消息([`compose_skill_prompt`])。
//!
//! 注:`disable-model-invocation` 是 Claude Code 的字段,opencode 视作未知字段
//! 忽略(实测带该字段的技能照样被登记),所以本模块不据此分类。
//!
//! 技能的创建/编辑/删除都是纯文件操作(opencode 没有任何 skills 子命令);
//! 改写时保留 name/description 之外的字段,并给会炸 YAML 的值补引号
//! —— 坏技能在编辑器里保存一次就修好了。删除走系统回收站(可找回)。

use crate::error::{CmdError, CmdResult};
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tauri::State;

type S<'a> = State<'a, Arc<AppState>>;

/// `opencode debug skill` 要加载插件与配置,给足冷启动时间。
const CATALOG_TIMEOUT: Duration = Duration::from_secs(90);
/// 项目级技能向上找几层(官方是找到 git 根为止,这里加个硬上限防呆)。
const PROJECT_WALK_UP_LIMIT: usize = 8;
/// 技能名规则(官方 SKILL.md 规范)。
const NAME_MAX: usize = 64;
const DESCRIPTION_MAX: usize = 1024;

/// 技能在界面上的分类状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillState {
    /// opencode 已登记,模型可自动调用
    Active,
    /// 磁盘上有但 opencode 没登记(frontmatter 不合规);仍可手动注入使用
    Unloaded,
}

#[derive(Debug, Clone, Serialize)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    /// SKILL.md 绝对路径;内置技能为空串
    pub path: String,
    /// builtin / global / project
    pub scope: String,
    /// 来源短标签(界面显示,如「全局 · .config/opencode」「内置」)
    pub source_label: String,
    pub state: SkillState,
    /// 内置技能不可编辑/删除
    pub editable: bool,
    /// 磁盘上同名副本的份数(技能常被同时装到 opencode/claude/agents 三处;
    /// 实际生效的只有一份,这里只提示,不列成多行)
    pub copies: u32,
    /// 未加载时的人话原因(能诊断出来才有值)
    pub reason: String,
}

#[derive(Debug, Serialize)]
pub struct SkillCatalog {
    pub skills: Vec<SkillInfo>,
    /// opencode 已登记(模型可自动调用)的数量
    pub active_count: usize,
    /// 取清单失败时的人话原因(此时 skills 仅来自文件扫描)
    pub warning: String,
}

/// 编辑器用的技能原文(正文 = 去掉 frontmatter 的部分)。
#[derive(Debug, Serialize)]
pub struct SkillSource {
    pub name: String,
    pub description: String,
    pub body: String,
}

/// `opencode debug skill` 的一条输出。
#[derive(Debug, Deserialize)]
struct DebugSkill {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    location: String,
}

// ------------------------------------------------------------- frontmatter

/// SKILL.md 的 frontmatter 关键字段(YAML 子集:每行 `key: value`,
/// 值里可能含冒号,只在第一个冒号处切;不支持多行折叠,官方样例也不用)。
/// `extras` 保留 name/description 之外的字段原样(如 Claude Code 的
/// `argument-hint`),改写技能时不能把用户的元数据弄丢。
#[derive(Debug, Default, PartialEq)]
pub struct FrontMatter {
    pub name: String,
    pub description: String,
    pub extras: Vec<(String, String)>,
}

/// 解析 frontmatter 与正文。没有 `---` 围栏时正文即全文。
pub fn parse_skill_md(text: &str) -> (FrontMatter, String) {
    // Windows 上的编辑器/PowerShell 常写出带 BOM 的 UTF-8;opencode 能读,
    // 我们也得能读,否则会把好技能误判成「缺 name」。
    let normalized = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let rest = match normalized.strip_prefix("---\n") {
        Some(r) => r,
        None => return (FrontMatter::default(), normalized.trim().to_string()),
    };
    let Some(end) = rest.find("\n---") else {
        return (FrontMatter::default(), normalized.trim().to_string());
    };
    let (front, after) = rest.split_at(end);
    let body = after
        .trim_start_matches('\n')
        .trim_start_matches("---")
        .trim_start_matches('\n')
        .trim()
        .to_string();

    let mut fm = FrontMatter::default();
    for line in front.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        let unquoted = value.trim_matches('"').trim_matches('\'').trim();
        match key {
            "name" => fm.name = unquoted.to_string(),
            "description" => fm.description = unquoted.to_string(),
            _ => fm.extras.push((key.to_string(), value.to_string())),
        }
    }
    (fm, body)
}

/// frontmatter 里会让 opencode 整份拒收的写法。
///
/// 实测(2026-08-19):`argument-hint: [稿子] [男声|女声]` —— YAML 流序列
/// 后面又跟一个 token,是语法错误,**整个技能会被丢弃**(对照实验:同一份
/// 技能删掉这行立刻被登记)。Claude Code 容忍这种写法,opencode 不容忍,
/// 所以从 Claude 那边装过来的技能很容易悄悄失效。
pub fn frontmatter_problem(fm: &FrontMatter) -> Option<String> {
    if fm.name.is_empty() {
        return Some("frontmatter 缺 name".into());
    }
    if fm.description.is_empty() {
        return Some("frontmatter 缺 description —— AI 看不见这个技能".into());
    }
    for (key, value) in &fm.extras {
        if needs_quoting(value) {
            return Some(format!("{key} 这一行的值要加引号,否则 YAML 解析失败"));
        }
    }
    None
}

/// 未加引号、且 YAML 会当成流序列/流映射来解析的值 —— 只有整行恰好是一个
/// 闭合的 `[...]`/`{...}` 才合法,其余(如 `[a] [b]`)都会报错。
fn needs_quoting(value: &str) -> bool {
    let v = value.trim();
    if v.starts_with('"') || v.starts_with('\'') {
        return false;
    }
    match v.chars().next() {
        Some('[') => !(v.ends_with(']') && v.matches('[').count() == 1),
        Some('{') => !(v.ends_with('}') && v.matches('{').count() == 1),
        _ => false,
    }
}

/// 组装 SKILL.md 全文(写入用)。保留 extras,并把会炸 YAML 的值补上引号
/// —— 用编辑器保存一次,从 Claude 那边搬来的坏技能就自动修好了。
pub fn build_skill_md(
    name: &str,
    description: &str,
    extras: &[(String, String)],
    body: &str,
) -> String {
    // description 必须单行:换行会破坏 frontmatter,直接压成空格
    let desc = description.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut front = format!("name: {name}\ndescription: {desc}\n");
    for (key, value) in extras {
        if needs_quoting(value) {
            let safe = value.replace('\\', "\\\\").replace('"', "\\\"");
            front.push_str(&format!("{key}: \"{safe}\"\n"));
        } else {
            front.push_str(&format!("{key}: {value}\n"));
        }
    }
    format!("---\n{front}---\n\n{}\n", body.trim())
}

/// 技能名规则校验(官方:小写字母数字 + 单连字符,1–64)。
pub fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > NAME_MAX {
        return Err(format!("技能名要 1–{NAME_MAX} 个字符"));
    }
    let ok = name.split('-').all(|seg| {
        !seg.is_empty()
            && seg
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
    });
    if !ok {
        return Err("技能名只能用小写英文、数字和单个连字符,例如 my-skill".into());
    }
    Ok(())
}

// ------------------------------------------------------------- catalog

fn skills_roots(home: &Path) -> [(PathBuf, &'static str); 3] {
    [
        (
            home.join(".config").join("opencode").join("skills"),
            "全局 · opencode",
        ),
        (home.join(".claude").join("skills"), "全局 · claude"),
        (home.join(".agents").join("skills"), "全局 · agents"),
    ]
}

/// 项目级技能根目录:从工作目录向上找(官方行为)。`seen` 里已有的路径
/// 跳过 —— 主目录下的 `.claude/.agents` 已经按"全局"扫过了,再当成
/// "上级目录"扫一遍只会产生重复行。
fn project_roots(workdir: &Path, seen: &HashSet<String>) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    let mut dir = Some(workdir);
    for _ in 0..PROJECT_WALK_UP_LIMIT {
        let Some(cur) = dir else { break };
        for sub in [".opencode", ".claude", ".agents"] {
            let root = cur.join(sub).join("skills");
            if !root.is_dir() || seen.contains(&root.to_string_lossy().to_lowercase()) {
                continue;
            }
            let label = if cur == workdir {
                format!("本文件夹 · {sub}")
            } else {
                format!("上级目录 · {sub}")
            };
            out.push((root, label));
        }
        // 官方:向上找到 git 工作区根为止
        if cur.join(".git").exists() {
            break;
        }
        dir = cur.parent();
    }
    out
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

/// 扫一个 skills 根目录下的所有 `*/SKILL.md`。
fn scan_root(root: &Path, scope: &str, label: &str, out: &mut Vec<SkillInfo>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let file = entry.path().join("SKILL.md");
        if !file.is_file() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let (fm, _) = parse_skill_md(&text);
        let problem = frontmatter_problem(&fm);
        // 名字缺失时退回目录名,至少让用户在面板里看得见、能修
        let name = if fm.name.is_empty() {
            entry.file_name().to_string_lossy().into_owned()
        } else {
            fm.name
        };
        out.push(SkillInfo {
            name,
            description: fm.description,
            path: file.to_string_lossy().into_owned(),
            scope: scope.to_string(),
            source_label: label.to_string(),
            state: SkillState::Unloaded,
            editable: true,
            copies: 1,
            reason: problem.unwrap_or_default(),
        });
    }
}

/// 问 opencode 要"已登记"的技能清单(权威去重结果)。
async fn debug_skill_list(state: &AppState, workdir: &Path) -> Result<Vec<DebugSkill>, String> {
    let (bin_override, proxy) = {
        let settings = state.settings.lock().expect("settings lock");
        (
            settings.ai.opencode_bin.clone(),
            settings
                .ai
                .proxy_url
                .clone()
                .filter(|p| !p.trim().is_empty()),
        )
    };
    let bin = sf_llm::channels::opencode::locate_binary(bin_override.as_deref().map(Path::new))
        .ok_or_else(|| "未找到 opencode——先到「设置 · AI 接入」一键安装".to_string())?;
    let mut cmd = sf_llm::channels::opencode::hidden_command(&bin, proxy.as_deref());
    let fut = cmd
        .args(["debug", "skill"])
        .current_dir(workdir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output();
    let out = tokio::time::timeout(CATALOG_TIMEOUT, fut)
        .await
        .map_err(|_| "opencode 读取技能清单超时".to_string())?
        .map_err(|e| format!("opencode 启动失败:{e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let start = stdout
        .find('[')
        .ok_or_else(|| "opencode 没有返回技能清单".to_string())?;
    serde_json::from_str::<Vec<DebugSkill>>(&stdout[start..])
        .map_err(|e| format!("技能清单解析失败:{e}"))
}

/// 技能总目录:opencode 已登记的 + 磁盘上没被登记的(带状态标注)。
#[tauri::command]
pub async fn skill_catalog(state: S<'_>, workdir: String) -> CmdResult<SkillCatalog> {
    let state = state.inner().clone();
    // 没有工作目录(未打开智能体会话)时用沙箱目录:只出全局与内置技能
    let cwd = if workdir.trim().is_empty() || !Path::new(&workdir).is_dir() {
        let dir = state.paths.agent_sandbox();
        std::fs::create_dir_all(&dir).ok();
        dir
    } else {
        PathBuf::from(&workdir)
    };

    // ① 先问 opencode 要权威清单
    let (loaded, warning) = match debug_skill_list(&state, &cwd).await {
        Ok(list) => (list, String::new()),
        Err(e) => (Vec::new(), e),
    };
    // ② 扫盘:全局三处 + 从工作目录向上的项目级(同一个根不重复扫)
    let mut disk: Vec<SkillInfo> = Vec::new();
    let mut scanned_roots: HashSet<String> = HashSet::new();
    if let Some(home) = home_dir() {
        for (root, label) in skills_roots(&home) {
            scanned_roots.insert(root.to_string_lossy().to_lowercase());
            scan_root(&root, "global", label, &mut disk);
        }
    }
    for (root, label) in project_roots(&cwd, &scanned_roots) {
        scan_root(&root, "project", &label, &mut disk);
    }
    let mut by_path: BTreeMap<String, SkillInfo> = BTreeMap::new();
    for d in disk {
        by_path.insert(d.path.to_lowercase(), d);
    }

    // ③ 按技能名归组:一个技能一行。同名副本很常见(技能安装器会把同一个
    //    技能同时装到 opencode/claude/agents 三处),真正生效的只有 opencode
    //    选中的那份 —— 列成多行只会让人以为装重了。
    let mut merged: BTreeMap<String, SkillInfo> = BTreeMap::new();
    let mut take = |mut info: SkillInfo| {
        let key = info.name.to_lowercase();
        match merged.get_mut(&key) {
            Some(existing) => {
                existing.copies += 1;
                // 保留状态最好的那份(可自动调用 > 手动触发 > 未加载)
                if info.state < existing.state {
                    info.copies = existing.copies;
                    *existing = info;
                }
            }
            None => {
                merged.insert(key, info);
            }
        }
    };
    // opencode 登记的先落座:那就是实际生效的那一份
    for s in &loaded {
        let builtin = s.location == "<built-in>";
        let scanned = (!builtin)
            .then(|| by_path.remove(&s.location.to_lowercase()))
            .flatten();
        take(SkillInfo {
            name: s.name.clone(),
            description: s.description.clone(),
            path: if builtin {
                String::new()
            } else {
                s.location.clone()
            },
            scope: if builtin {
                "builtin".into()
            } else {
                scanned
                    .as_ref()
                    .map(|x| x.scope.clone())
                    .unwrap_or_else(|| "global".into())
            },
            source_label: if builtin {
                "内置".into()
            } else {
                scanned
                    .as_ref()
                    .map(|x| x.source_label.clone())
                    .unwrap_or_else(|| "已加载".into())
            },
            state: SkillState::Active,
            editable: !builtin,
            copies: 1,
            reason: String::new(),
        });
    }
    for (_, s) in by_path {
        take(s);
    }

    // 排序:可用的在前,其次手动触发,最后未加载;组内按名字
    let mut skills: Vec<SkillInfo> = merged.into_values().collect();
    skills.sort_by(|a, b| {
        a.state
            .cmp(&b.state)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    let active_count = skills
        .iter()
        .filter(|s| s.state == SkillState::Active)
        .count();
    Ok(SkillCatalog {
        skills,
        active_count,
        warning,
    })
}

// ------------------------------------------------------------- read / write

/// 读技能原文供编辑。
#[tauri::command]
pub fn skill_source(path: String) -> CmdResult<SkillSource> {
    let text = std::fs::read_to_string(&path)
        .map_err(|e| CmdError::new("skill", format!("读不到这个技能文件:{e}")))?;
    let (fm, body) = parse_skill_md(&text);
    Ok(SkillSource {
        name: fm.name,
        description: fm.description,
        body,
    })
}

/// 新建或改写一个技能。`path` 非空 = 改写已有技能(名字不可变)。
#[tauri::command]
pub fn skill_save(
    state: S<'_>,
    path: String,
    scope: String,
    workdir: String,
    name: String,
    description: String,
    body: String,
) -> CmdResult<String> {
    let name = name.trim().to_lowercase();
    validate_name(&name).map_err(|e| CmdError::new("bad_name", e))?;
    let description = description.trim();
    if description.is_empty() {
        return Err(CmdError::new(
            "bad_description",
            "「什么时候用它」不能为空——AI 就是靠这句话决定要不要用这个技能的",
        ));
    }
    if description.chars().count() > DESCRIPTION_MAX {
        return Err(CmdError::new(
            "bad_description",
            format!("「什么时候用它」最多 {DESCRIPTION_MAX} 字"),
        ));
    }
    if body.trim().is_empty() {
        return Err(CmdError::new("bad_body", "技能指令不能为空"));
    }

    // 改写已有技能时保留 name/description 之外的字段(如 argument-hint);
    // 写回时坏 YAML 会被自动补引号 —— 编辑一次即修好
    let extras = if path.trim().is_empty() {
        Vec::new()
    } else {
        std::fs::read_to_string(&path)
            .map(|raw| parse_skill_md(&raw).0.extras)
            .unwrap_or_default()
    };
    let file = if path.trim().is_empty() {
        // 新建:按作用范围决定落到哪个根目录
        let dir = match scope.as_str() {
            "project" => {
                let wd = Path::new(&workdir);
                if workdir.trim().is_empty() || !wd.is_dir() {
                    return Err(CmdError::new(
                        "bad_workdir",
                        "「只在这个文件夹用」需要先打开一个智能体会话",
                    ));
                }
                wd.join(".opencode").join("skills").join(&name)
            }
            _ => {
                let home = home_dir().ok_or_else(|| CmdError::new("skill", "找不到用户主目录"))?;
                home.join(".config")
                    .join("opencode")
                    .join("skills")
                    .join(&name)
            }
        };
        if dir.join("SKILL.md").exists() {
            return Err(CmdError::new(
                "exists",
                format!("已经有一个叫「{name}」的技能了,换个名字吧"),
            ));
        }
        std::fs::create_dir_all(&dir)
            .map_err(|e| CmdError::new("skill", format!("创建技能目录失败:{e}")))?;
        dir.join("SKILL.md")
    } else {
        PathBuf::from(&path)
    };

    std::fs::write(&file, build_skill_md(&name, description, &extras, &body))
        .map_err(|e| CmdError::new("skill", format!("写入技能失败:{e}")))?;
    let _ = state; // 保留 state 形参:后续可能要在这里做缓存失效
    Ok(file.to_string_lossy().into_owned())
}

/// 删除技能:整个技能目录移入系统回收站(可找回)。
#[tauri::command]
pub fn skill_delete(path: String) -> CmdResult<String> {
    let file = PathBuf::from(&path);
    if file.file_name().and_then(|n| n.to_str()) != Some("SKILL.md") {
        return Err(CmdError::new("skill", "这不是一个技能文件"));
    }
    let dir = file
        .parent()
        .ok_or_else(|| CmdError::new("skill", "技能路径不完整"))?;
    // 只允许删 `<某个 skills 根>/<技能名>/` 这一层,防止误伤上层目录
    let parent_ok = dir
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .map(|n| n.eq_ignore_ascii_case("skills"))
        .unwrap_or(false);
    if !parent_ok {
        return Err(CmdError::new(
            "skill",
            "这个技能不在标准的 skills 目录里,请手动处理",
        ));
    }
    trash::delete(dir).map_err(|e| CmdError::new("skill", format!("删除失败:{e}")))?;
    Ok(dir.to_string_lossy().into_owned())
}

/// 把技能正文注入一条消息(手动触发型技能的等价触发方式)。
/// 形状对齐 opencode 自己 skill 工具的输出,模型见到就知道该照办。
pub fn compose_skill_prompt(name: &str, body: &str, user_text: &str) -> String {
    format!(
        "<skill_content name=\"{name}\">\n{}\n</skill_content>\n\n\
         Follow the skill above for this task:\n{user_text}",
        body.trim()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_parses_name_description_and_manual_flag() {
        let text = "---\nname: cn-explainer-video\ndescription: 手动触发(/cn-explainer-video):做中文口播视频,参数:声线、字幕\nargument-hint: [稿子]\ndisable-model-invocation: true\n---\n\n# 正文标题\n\n步骤一\n";
        let (fm, body) = parse_skill_md(text);
        assert_eq!(fm.name, "cn-explainer-video");
        // 描述里含多个冒号:只在第一个冒号处切
        assert!(
            fm.description
                .starts_with("手动触发(/cn-explainer-video):做中文口播视频")
        );
        // name/description 之外的字段原样留着,改写技能时不能丢
        assert_eq!(
            fm.extras,
            vec![
                ("argument-hint".to_string(), "[稿子]".to_string()),
                ("disable-model-invocation".to_string(), "true".to_string()),
            ]
        );
        assert!(body.starts_with("# 正文标题"));
        assert!(body.ends_with("步骤一"));
    }

    /// 真机对照实验(2026-08-19):`argument-hint: [a] [b]` 是非法 YAML,
    /// opencode 整份技能拒收;删掉该行同一技能立刻被登记。
    #[test]
    fn broken_yaml_value_is_diagnosed_and_fixed_on_save() {
        let text = "---\nname: cn-video\ndescription: 做视频\nargument-hint: [稿子/素材] [男声|女声]\n---\n\nbody\n";
        let (fm, body) = parse_skill_md(text);
        let problem = frontmatter_problem(&fm).expect("应诊断出 YAML 问题");
        assert!(problem.contains("argument-hint"));
        assert!(problem.contains("引号"));

        // 保存一次即修好:值补引号,其余字段照旧
        let fixed = build_skill_md(&fm.name, &fm.description, &fm.extras, &body);
        assert!(fixed.contains("argument-hint: \"[稿子/素材] [男声|女声]\"\n"));
        let (fm2, _) = parse_skill_md(&fixed);
        assert!(frontmatter_problem(&fm2).is_none());
    }

    #[test]
    fn well_formed_values_are_left_alone() {
        assert!(!needs_quoting("[a, b]")); // 单个闭合流序列是合法 YAML
        assert!(!needs_quoting("\"[a] [b]\""));
        assert!(!needs_quoting("plain text"));
        assert!(needs_quoting("[a] [b]"));
        assert!(needs_quoting("{a} {b}"));
    }

    #[test]
    fn missing_fields_are_reported() {
        let (fm, _) = parse_skill_md("---\nname: x\n---\n\nbody");
        assert!(frontmatter_problem(&fm).unwrap().contains("description"));
        let (fm2, _) = parse_skill_md("---\ndescription: d\n---\n\nbody");
        assert!(frontmatter_problem(&fm2).unwrap().contains("name"));
    }

    #[test]
    fn frontmatter_defaults_when_absent() {
        let (fm, body) = parse_skill_md("# 只有正文\n\n内容");
        assert_eq!(fm, FrontMatter::default());
        assert!(fm.extras.is_empty());
        assert_eq!(body, "# 只有正文\n\n内容");
    }

    /// PowerShell 的 `Set-Content -Encoding utf8` 会加 BOM(真机踩坑:
    /// 带 BOM 的技能被误判成「缺 name」)。
    #[test]
    fn utf8_bom_is_stripped() {
        let text = "\u{feff}---\nname: bom-skill\ndescription: works\n---\n\nbody";
        let (fm, body) = parse_skill_md(text);
        assert_eq!(fm.name, "bom-skill");
        assert_eq!(fm.description, "works");
        assert_eq!(body, "body");
        assert!(frontmatter_problem(&fm).is_none());
    }

    #[test]
    fn crlf_and_quotes_are_tolerated() {
        let text =
            "---\r\nname: \"my-skill\"\r\ndescription: 'when to use'\r\n---\r\n\r\nbody line\r\n";
        let (fm, body) = parse_skill_md(text);
        assert_eq!(fm.name, "my-skill");
        assert_eq!(fm.description, "when to use");
        assert_eq!(body, "body line");
    }

    #[test]
    fn build_then_parse_roundtrips() {
        let md = build_skill_md(
            "tea-brewing",
            "Use when brewing tea",
            &[],
            "# Steps\n\n1. Boil",
        );
        let (fm, body) = parse_skill_md(&md);
        assert_eq!(fm.name, "tea-brewing");
        assert_eq!(fm.description, "Use when brewing tea");
        assert_eq!(body, "# Steps\n\n1. Boil");
    }

    #[test]
    fn multiline_description_is_flattened() {
        let md = build_skill_md("x", "第一行\n第二行", &[], "body");
        assert!(md.contains("description: 第一行 第二行\n"));
        let (fm, _) = parse_skill_md(&md);
        assert_eq!(fm.description, "第一行 第二行");
    }

    #[test]
    fn name_rules_match_official_regex() {
        for ok in ["a", "my-skill", "skill2", "a1-b2-c3"] {
            assert!(validate_name(ok).is_ok(), "{ok} should pass");
        }
        for bad in [
            "",
            "My-Skill",
            "my_skill",
            "-lead",
            "trail-",
            "double--dash",
            "空格 名",
        ] {
            assert!(validate_name(bad).is_err(), "{bad} should fail");
        }
        assert!(validate_name(&"a".repeat(65)).is_err());
    }

    /// 归组规则:状态越好越优先(Active < ManualOnly < Unloaded 的序即优先级)。
    #[test]
    fn state_ordering_prefers_the_usable_copy() {
        assert!(SkillState::Active < SkillState::Unloaded);
        let mut states = [SkillState::Unloaded, SkillState::Active];
        states.sort();
        assert_eq!(states[0], SkillState::Active);
    }

    #[test]
    fn skill_prompt_wraps_body_and_task() {
        let p = compose_skill_prompt("tea", "Steep 97s", "帮我泡茶");
        assert!(p.starts_with("<skill_content name=\"tea\">"));
        assert!(p.contains("Steep 97s"));
        assert!(p.trim_end().ends_with("帮我泡茶"));
    }
}
