//! 下载目录监听器:向导等待素材期间监听系统下载目录,
//! 新增图片/视频文件在**大小稳定**后经 `pet://watcher-file` 播报给前端自动导入。
//!
//! 隐私边界(与「素材不出本机」逐字对齐):
//! * 默认**关闭**(`settings.pet.watcher_enabled`,句流语境下的保守化);
//! * 即便打开,也仅在前端显式调用 `pet_watcher_start`(向导处于等待素材步骤)期间生效;
//! * 只上报文件路径事件,不读取、不落盘、不上传任何内容。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::error::{CmdError, CmdResult};

const IMAGE_EXTS: [&str; 6] = ["png", "jpg", "jpeg", "webp", "bmp", "gif"];
const VIDEO_EXTS: [&str; 3] = ["mp4", "mov", "webm"];
/// 浏览器下载中间态后缀,忽略。
const PARTIAL_EXTS: [&str; 4] = ["crdownload", "part", "tmp", "download"];
/// 同一路径 30s 内只上报一次。
const DEDUP_WINDOW: Duration = Duration::from_secs(30);
/// 文件写完判据:连续两次采样大小相同且非零,最多等 8s。
const STABLE_POLL: Duration = Duration::from_millis(400);
const STABLE_TRIES: usize = 20;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatcherFile {
    pub path: String,
    /// "image" | "video"
    pub kind: String,
}

#[derive(Default)]
pub struct WatcherState {
    watcher: Mutex<Option<RecommendedWatcher>>,
    recent: Mutex<HashMap<PathBuf, Instant>>,
    /// 代次旗标:`stop` 后仍在等待文件稳定的短线程据此闭嘴。
    ///
    /// 原实现里那些线程最长还会活 8 秒,停掉监听后仍可能弹出一条
    /// 「检测到新素材」—— 用户以为已经关掉了。
    active: AtomicBool,
}

impl WatcherState {
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::SeqCst)
    }
}

fn classify(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    if IMAGE_EXTS.contains(&ext.as_str()) {
        Some("image")
    } else if VIDEO_EXTS.contains(&ext.as_str()) {
        Some("video")
    } else {
        None
    }
}

fn is_partial(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| PARTIAL_EXTS.contains(&e.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// 等待文件写完:连续两次采样大小相同且非零。
fn wait_stable(path: &Path) -> bool {
    let mut last: u64 = 0;
    for _ in 0..STABLE_TRIES {
        std::thread::sleep(STABLE_POLL);
        let Ok(meta) = std::fs::metadata(path) else {
            continue;
        };
        let size = meta.len();
        if size > 0 && size == last {
            return true;
        }
        last = size;
    }
    false
}

fn handle_path(app: &AppHandle, state: &WatcherState, path: PathBuf) {
    if is_partial(&path) {
        return;
    }
    let Some(kind) = classify(&path) else { return };
    {
        let mut recent = state.recent.lock().expect("pet watcher recent lock");
        let now = Instant::now();
        recent.retain(|_, t| now.duration_since(*t) < DEDUP_WINDOW);
        if recent.contains_key(&path) {
            return;
        }
        recent.insert(path.clone(), now);
    }
    // 稳定性等待放到独立线程,不阻塞 notify 回调
    let app = app.clone();
    std::thread::spawn(move || {
        if !wait_stable(&path) {
            return;
        }
        // 等待期间用户可能已经关掉监听 —— 关掉之后不该再冒出提示
        let state = crate::pet::state_of(&app);
        if !state.pet.watcher.is_active() {
            return;
        }
        let _ = app.emit(
            crate::pet::EV_WATCHER_FILE,
            WatcherFile {
                path: path.to_string_lossy().to_string(),
                kind: kind.into(),
            },
        );
    });
}

/// 启动监听(幂等:重复调用先停旧的)。
pub fn start(app: &AppHandle, state: &WatcherState) -> CmdResult<()> {
    use tauri::Manager;
    let downloads = app
        .path()
        .download_dir()
        .map_err(|_| CmdError::new("pet", "无法定位下载目录"))?;

    let cb_app = app.clone();
    let mut watcher =
        notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| {
            let Ok(event) = res else { return };
            if !matches!(
                event.kind,
                notify::EventKind::Create(_) | notify::EventKind::Modify(_)
            ) {
                return;
            }
            let state = crate::pet::state_of(&cb_app);
            for path in event.paths {
                handle_path(&cb_app, &state.pet.watcher, path);
            }
        })
        .map_err(|e| CmdError::new("pet", format!("监听器启动失败: {e}")))?;
    watcher
        .watch(&downloads, RecursiveMode::NonRecursive)
        .map_err(|e| CmdError::new("pet", format!("无法监听下载目录: {e}")))?;

    state.active.store(true, Ordering::SeqCst);
    *state.watcher.lock().expect("pet watcher lock") = Some(watcher);
    Ok(())
}

/// 停止监听(drop watcher 即解除)。
pub fn stop(state: &WatcherState) {
    state.active.store(false, Ordering::SeqCst);
    *state.watcher.lock().expect("pet watcher lock") = None;
    state
        .recent
        .lock()
        .expect("pet watcher recent lock")
        .clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_and_partial_rules() {
        assert_eq!(classify(Path::new("a/b/pic.PNG")), Some("image"));
        assert_eq!(classify(Path::new("clip.mp4")), Some("video"));
        assert_eq!(classify(Path::new("doc.pdf")), None);
        assert_eq!(classify(Path::new("noext")), None);
        assert!(is_partial(Path::new("pic.png.crdownload")));
        assert!(!is_partial(Path::new("pic.png")));
    }

    /// `stop` 之后的活跃旗标必须立刻落下:等待文件稳定的短线程靠它闭嘴,
    /// 否则关掉监听后最长 8 秒内仍会弹出「检测到新素材」。
    #[test]
    fn stop_clears_the_active_flag_immediately() {
        let state = WatcherState::default();
        assert!(!state.is_active(), "默认未启动");
        state.active.store(true, Ordering::SeqCst);
        state
            .recent
            .lock()
            .unwrap()
            .insert(PathBuf::from("x.png"), Instant::now());
        stop(&state);
        assert!(!state.is_active());
        assert!(state.recent.lock().unwrap().is_empty(), "去重表应一并清空");
    }
}
