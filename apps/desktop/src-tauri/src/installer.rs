//! opencode 一键安装器(§4.7 引导优化,面向不会用命令行的小白用户)。
//!
//! 不依赖 Node.js、不弹终端:opencode 官方把各平台二进制发布为 npm
//! 平台包(如 `opencode-windows-x64`,内含单个 `bin/opencode.exe`)。
//! 这里直接从 npm registry 下载 tgz → 解出 exe → 放进应用数据目录
//! `<app-data>/tools/opencode/` → 健康检查(--version)→ 写入
//! `settings.ai.opencode_bin`,探测层(bin_override)即刻生效。
//!
//! registry 候选来自 channels.json(`npm_registries`,国内镜像在前,
//! 数据驱动可随内容包更新);下载遵循用户配置的 AI 代理。
//! 进度经 `install://progress` 事件流式上报。

use crate::error::{CmdError, CmdResult};
use crate::state::AppState;
use futures::StreamExt;
use serde::Serialize;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

#[derive(Debug, Clone, Serialize)]
pub struct InstallProgress {
    /// resolve | download | extract | verify
    pub phase: String,
    pub received: u64,
    pub total: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallDone {
    pub version: String,
    pub bin_path: String,
    /// 安装目录是否已加入用户 PATH(新开的终端可直接用 `opencode` 命令)。
    pub on_path: bool,
}

/// opencode(Bun 构建)依赖 Win10 1809 引入的 ConPTY API
/// (ClosePseudoConsole 等);更老的系统上加载即失败。
#[cfg(windows)]
const MIN_WINDOWS_BUILD: u32 = 17763;

/// 真实 Windows build 号(RtlGetVersion 不受兼容性清单影响);
/// 探测失败返回 0(未知时不拦安装)。
#[cfg(windows)]
fn windows_build_number() -> u32 {
    #[repr(C)]
    struct OsVersionInfo {
        size: u32,
        major: u32,
        minor: u32,
        build: u32,
        platform_id: u32,
        csd: [u16; 128],
    }
    #[link(name = "ntdll")]
    unsafe extern "system" {
        fn RtlGetVersion(info: *mut OsVersionInfo) -> i32;
    }
    let mut info = OsVersionInfo {
        size: std::mem::size_of::<OsVersionInfo>() as u32,
        major: 0,
        minor: 0,
        build: 0,
        platform_id: 0,
        csd: [0; 128],
    };
    if unsafe { RtlGetVersion(&mut info) } == 0 {
        info.build
    } else {
        0
    }
}

const OLD_WINDOWS_MSG: &str = "这台电脑的 Windows 版本过旧,opencode 无法在其上运行\
(需要 Windows 10 的 2018 年 10 月更新 1809 及以上)。建议改用「Zen 直连」通道\
——无需安装任何东西,填 Key 即可;或升级 Windows 后再来一键安装。";

/// 安装目录:标准的用户级程序位置(免管理员权限,同时也在探测器的
/// 兜底搜索列表里)。取不到 LOCALAPPDATA 时退回应用数据目录。
fn install_dir(state: &AppState) -> PathBuf {
    #[cfg(windows)]
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        return PathBuf::from(local).join("Programs").join("opencode");
    }
    state.paths.root.join("tools").join("opencode")
}

/// 把安装目录追加进用户 PATH(HKCU,不动系统 PATH、免管理员)。
/// 用隐藏的 PowerShell 跑 .NET SetEnvironmentVariable:读改写原子,
/// 且自动广播 WM_SETTINGCHANGE——之后新开的终端直接可用 `opencode`。
/// 失败不致命(应用自身走 opencode_bin,不依赖 PATH)。
#[cfg(windows)]
fn add_to_user_path(dir: &std::path::Path) -> bool {
    use std::os::windows::process::CommandExt;
    let dir_str = dir.to_string_lossy().replace('\'', "''");
    let script = format!(
        "$d = '{dir_str}'; \
         $p = [Environment]::GetEnvironmentVariable('Path', 'User'); \
         if ($null -eq $p) {{ $p = '' }}; \
         if (($p -split ';') -notcontains $d) {{ \
             [Environment]::SetEnvironmentVariable('Path', ($p.TrimEnd(';') + ';' + $d).TrimStart(';'), 'User') \
         }}"
    );
    std::process::Command::new("powershell")
        .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &script])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 当前平台对应的 opencode npm 平台包;不支持的平台返回 None
/// (UI 只在 Windows 上展示一键安装)。
fn platform_package() -> Option<&'static str> {
    if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        Some("opencode-windows-x64")
    } else if cfg!(all(target_os = "windows", target_arch = "aarch64")) {
        Some("opencode-windows-arm64")
    } else {
        None
    }
}

fn emit_progress(app: &AppHandle, phase: &str, received: u64, total: Option<u64>) {
    let _ = app.emit(
        "install://progress",
        InstallProgress {
            phase: phase.into(),
            received,
            total,
        },
    );
}

fn http_client(proxy: Option<&str>) -> Result<reqwest::Client, CmdError> {
    let mut b = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .user_agent("sentenceflow-desktop");
    if let Some(p) = proxy {
        b = b.proxy(
            reqwest::Proxy::all(p)
                .map_err(|e| CmdError::new("install", format!("代理地址无效: {e}")))?,
        );
    }
    b.build()
        .map_err(|e| CmdError::new("install", format!("网络组件初始化失败: {e}")))
}

/// 逐个 registry 询问 opencode 最新版本号。
async fn resolve_latest(client: &reqwest::Client, registries: &[String]) -> Result<String, String> {
    let mut last_err = String::new();
    for reg in registries {
        let url = format!("{}/opencode-ai/latest", reg.trim_end_matches('/'));
        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => {
                match resp.json::<serde_json::Value>().await {
                    Ok(v) => {
                        if let Some(ver) = v.get("version").and_then(|x| x.as_str()) {
                            return Ok(ver.to_string());
                        }
                        last_err = format!("{reg}: 响应缺少 version 字段");
                    }
                    Err(e) => last_err = format!("{reg}: {e}"),
                }
            }
            Ok(resp) => last_err = format!("{reg}: HTTP {}", resp.status()),
            Err(e) => last_err = format!("{reg}: {e}"),
        }
    }
    Err(last_err)
}

/// 流式下载 tgz 到 `dest`,边下边发进度。
async fn download_tgz(
    app: &AppHandle,
    client: &reqwest::Client,
    url: &str,
    dest: &PathBuf,
) -> Result<(), String> {
    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("{url}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("{url}: HTTP {}", resp.status()));
    }
    let total = resp.content_length();
    let mut file = std::fs::File::create(dest).map_err(|e| format!("写入临时文件失败: {e}"))?;
    let mut stream = resp.bytes_stream();
    let mut received = 0u64;
    let mut last_emit = 0u64;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("下载中断: {e}"))?;
        file.write_all(&chunk).map_err(|e| format!("写入失败: {e}"))?;
        received += chunk.len() as u64;
        // 每 512KB 上报一次,避免事件风暴
        if received - last_emit >= 512 * 1024 {
            emit_progress(app, "download", received, total);
            last_emit = received;
        }
    }
    emit_progress(app, "download", received, total);
    Ok(())
}

/// 从 tgz 中解出 `bin/opencode.exe` 到目标路径。
fn extract_binary(tgz: &PathBuf, bin_dest: &PathBuf) -> Result<(), String> {
    let file = std::fs::File::open(tgz).map_err(|e| format!("打开安装包失败: {e}"))?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(gz);
    for entry in archive.entries().map_err(|e| format!("安装包损坏: {e}"))? {
        let mut entry = entry.map_err(|e| format!("安装包损坏: {e}"))?;
        let path = entry.path().map_err(|e| e.to_string())?.into_owned();
        let name = path.to_string_lossy().replace('\\', "/");
        if name.ends_with("bin/opencode.exe") || name.ends_with("bin/opencode") {
            entry
                .unpack(bin_dest)
                .map_err(|e| format!("解包失败: {e}"))?;
            return Ok(());
        }
    }
    Err("安装包里没有找到 opencode 可执行文件".into())
}

/// 运行 `opencode --version` 做健康检查(隐藏窗口),返回版本输出。
fn health_check(bin: &PathBuf) -> Result<String, String> {
    let mut cmd = std::process::Command::new(bin);
    cmd.arg("--version");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let out = cmd
        .output()
        .map_err(|e| format!("安装后的程序无法运行: {e}"))?;
    if !out.status.success() {
        // STATUS_ENTRYPOINT_NOT_FOUND / STATUS_INVALID_IMAGE_FORMAT:
        // 系统太旧或架构不符,给可行动的中文解释而非十六进制码
        let code = out.status.code().unwrap_or(0) as u32;
        if code == 0xC000_0139 || code == 0xC000_007B {
            return Err(OLD_WINDOWS_MSG.into());
        }
        return Err(format!(
            "安装后的程序自检失败: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// 一键安装主流程。已装同版本时短路(仅健康检查 + 写设置)。
pub async fn install(app: AppHandle, state: Arc<AppState>) -> CmdResult<InstallDone> {
    let Some(pkg) = platform_package() else {
        return Err(CmdError::new(
            "install",
            "当前系统请使用命令行安装 opencode",
        ));
    };
    // 预检系统版本:过旧系统直接给出路,不浪费 57MB 下载(真机踩坑:
    // 老 Win10 上加载 opencode 报"无法定位程序输入点 ClosePseudoConsole")
    #[cfg(windows)]
    {
        let build = windows_build_number();
        if build != 0 && build < MIN_WINDOWS_BUILD {
            return Err(CmdError::new("install", OLD_WINDOWS_MSG));
        }
    }
    let (registries, proxy) = {
        let settings = state.settings.lock().expect("settings lock");
        (
            state.policy.npm_registries.clone(),
            settings
                .ai
                .proxy_url
                .clone()
                .filter(|p| !p.trim().is_empty()),
        )
    };
    let registries = if registries.is_empty() {
        sf_llm::policy::ChannelPolicy::default().npm_registries
    } else {
        registries
    };
    let client = http_client(proxy.as_deref())?;

    emit_progress(&app, "resolve", 0, None);
    let version = resolve_latest(&client, &registries)
        .await
        .map_err(|e| CmdError::new("install", format!("获取版本信息失败(检查网络):{e}")))?;
    if state
        .policy
        .opencode_known_bad
        .iter()
        .any(|bad| !bad.is_empty() && version.contains(bad.as_str()))
    {
        return Err(CmdError::new(
            "install",
            format!("最新版 {version} 存在已知问题,请过几天再试"),
        ));
    }

    let dir = install_dir(&state);
    std::fs::create_dir_all(&dir).map_err(|e| CmdError::new("install", format!("创建目录失败: {e}")))?;
    let bin = dir.join(if cfg!(windows) { "opencode.exe" } else { "opencode" });
    let marker = dir.join("version.txt");

    // 已装同版本 → 短路
    let installed = std::fs::read_to_string(&marker).unwrap_or_default();
    if !(bin.exists() && installed.trim() == version) {
        let tgz = dir.join("download.tgz");
        let mut last_err = String::new();
        let mut ok = false;
        for reg in &registries {
            let url = format!(
                "{}/{pkg}/-/{pkg}-{version}.tgz",
                reg.trim_end_matches('/')
            );
            match download_tgz(&app, &client, &url, &tgz).await {
                Ok(()) => {
                    ok = true;
                    break;
                }
                Err(e) => last_err = e,
            }
        }
        if !ok {
            let _ = std::fs::remove_file(&tgz);
            return Err(CmdError::new(
                "install",
                format!("下载失败(检查网络后重试):{last_err}"),
            ));
        }
        emit_progress(&app, "extract", 0, None);
        extract_binary(&tgz, &bin).map_err(|e| CmdError::new("install", e))?;
        let _ = std::fs::remove_file(&tgz);
        std::fs::write(&marker, &version)
            .map_err(|e| CmdError::new("install", format!("写入版本标记失败: {e}")))?;
    }

    emit_progress(&app, "verify", 0, None);
    if let Err(e) = health_check(&bin) {
        // 自检不过就清掉残件,避免后续探测反复触碰坏二进制
        let _ = std::fs::remove_file(&bin);
        let _ = std::fs::remove_file(&marker);
        return Err(CmdError::new("install", e));
    }

    // 全局可用:安装目录写入用户 PATH(新终端可直接敲 `opencode`,
    // 兼容有经验的用户);失败不影响应用内使用
    #[cfg(windows)]
    let on_path = add_to_user_path(&dir);
    #[cfg(not(windows))]
    let on_path = false;

    // 迁移清理:早期版本装在应用数据目录,统一挪到标准位置后删掉旧副本
    let legacy = state.paths.root.join("tools").join("opencode");
    if legacy != dir && legacy.exists() {
        let _ = std::fs::remove_dir_all(&legacy);
    }

    // 写入设置:探测/生成/答疑全部经 bin_override 使用这份托管副本
    {
        let mut settings = state.settings.lock().expect("settings lock");
        settings.ai.opencode_bin = Some(bin.to_string_lossy().to_string());
        let snapshot = settings.clone();
        drop(settings);
        state.save_settings(&snapshot)?;
    }

    Ok(InstallDone {
        version,
        bin_path: bin.to_string_lossy().to_string(),
        on_path,
    })
}

/// 打开一个独立控制台窗口运行 `opencode auth login`,小白按提示完成登录
/// (方向键选择 + 回车),关掉窗口后回软件点「重新检测」。
pub fn login(state: &AppState) -> CmdResult<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        let bin = {
            let settings = state.settings.lock().expect("settings lock");
            settings
                .ai
                .opencode_bin
                .clone()
                .unwrap_or_else(|| "opencode".into())
        };
        std::process::Command::new(&bin)
            .args(["auth", "login"])
            .creation_flags(CREATE_NEW_CONSOLE)
            .spawn()
            .map_err(|e| CmdError::new("install", format!("无法打开登录窗口: {e}")))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = state;
        Err(CmdError::new(
            "install",
            "当前系统请在终端运行 opencode auth login",
        ))
    }
}
