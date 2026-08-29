//! AI 萌宠 —— 桌面壳胶水层。
//!
//! 纯逻辑全在 [`sf_pet`](../../../../crates/sf-pet/src/lib.rs);本模块只做四件事:
//! 取参 → 调内核 → 落盘 → 广播事件。
//!
//! ## 隔离约定(《AI 萌宠模块-整合执行方案》§3.1)
//!
//! * **只做加法**:句流练习/句库/AI 等既有模块不因本模块产生行为变化;
//!   `sf-core`/`sf-pipeline`/`sf-llm` 一行不改。
//! * **命名空间**:命令一律 `pet_*`,事件一律 `pet://*`,数据在 `<app_data>/pet/`,
//!   前端路由 key `aipet` —— 与既有命令/事件零碰撞。
//! * **默认关闭**:`settings.pet.enabled` 默认 `false`,此时不建窗、不起线程,
//!   打包产物的行为与整合前逐项一致。
//!
//! ## 事件面
//!
//! | 事件 | 时机 |
//! |---|---|
//! | `pet://changed` | 图集/激活宠物变更(孵化/进化/校准/删除) |
//! | `pet://settings` | pet 设置分节快照 |
//! | `pet://theme` | 句流外观变更(主题跟随) |
//! | `pet://reminder` | 提醒/番茄钟到点 |
//! | `pet://pomodoro` | 番茄钟 1Hz 心跳 |
//! | `pet://export-progress` | 出生视频编码进度 |
//! | `pet://video-progress` | 视频抽帧/抠像进度 |
//! | `pet://watcher-file` | 下载目录发现新素材 |
//! | `pet://nav` | 主窗「AI 萌宠」页内切签 |
//!
//! (HatchDesk 的 `click-through-changed` 不迁移:全局穿透开关是 pet 设置的一个
//! 字段,`pet://settings` 已携带全量快照,再来一条只会制造第二个事实源。)

pub mod commands;
mod paths;
mod reminder;
pub mod runtime;
mod state;
mod watcher;

use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager};

use crate::error::{CmdError, CmdResult};
use crate::settings::{PetSettings, Theme};
use crate::state::AppState;

pub use state::PetState;

// ---------------------------------------------------------------- 事件名

pub const EV_CHANGED: &str = "pet://changed";
pub const EV_SETTINGS: &str = "pet://settings";
pub const EV_THEME: &str = "pet://theme";
pub const EV_REMINDER: &str = "pet://reminder";
pub const EV_POMODORO: &str = "pet://pomodoro";
pub const EV_EXPORT_PROGRESS: &str = "pet://export-progress";
pub const EV_VIDEO_PROGRESS: &str = "pet://video-progress";
pub const EV_WATCHER_FILE: &str = "pet://watcher-file";
pub const EV_NAV: &str = "pet://nav";

// ---------------------------------------------------------------- 错误转译

/// 内核错误 → 命令错误。内核的文案已是人话,原样转交前端(§11.E)。
impl From<sf_pet::PetError> for CmdError {
    fn from(e: sf_pet::PetError) -> Self {
        CmdError::new("pet", e.to_string())
    }
}

// ---------------------------------------------------------------- 状态取用

/// 从 `AppHandle` 取 pet 状态。后台回调(watcher / 心跳)没有 `State<'_>` 可用。
pub(crate) fn state_of(app: &AppHandle) -> Arc<AppState> {
    app.state::<Arc<AppState>>().inner().clone()
}

/// pet 设置的只读快照。**取完即释放句流的 settings 锁**,绝不带着它去动 pet 状态。
pub(crate) fn settings_snapshot(app: &AppHandle) -> PetSettings {
    state_of(app)
        .settings
        .lock()
        .expect("settings lock")
        .pet
        .clone()
}

// ---------------------------------------------------------------- 装配

/// 进程启动时的 pet 装配:仅当总开关打开才建窗 + 起心跳。
///
/// 默认关闭 ⇒ 老用户升级后这里什么都不做,整合对既有功能零影响。
pub fn setup(app: &AppHandle) {
    let settings = settings_snapshot(app);
    if !settings.enabled {
        return;
    }
    if let Err(e) = enable(app, &settings) {
        // 宠物起不来不该拖垮主程序:记日志,主窗照常。
        tracing::warn!("AI 萌宠启动失败(不影响主程序): {e}");
    }
}

/// 打开宠物:建窗 + 起心跳。
fn enable(app: &AppHandle, settings: &PetSettings) -> CmdResult<()> {
    runtime::ensure_window(app, settings)?;
    state_of(app).pet.start_heartbeat(app.clone());
    Ok(())
}

/// 关闭宠物:销毁窗口 + 停心跳 + 停下载监听。
fn disable(app: &AppHandle) {
    runtime::close_window(app);
    let state = state_of(app);
    state.pet.stop_heartbeat();
    watcher::stop(&state.pet.watcher);
}

/// 进程退出前收干净后台执行体。
pub fn shutdown(app: &AppHandle) {
    let state = state_of(app);
    state.pet.stop_heartbeat();
    watcher::stop(&state.pet.watcher);
}

/// 窗口事件分派(句流原本没有 `on_window_event`,本处理器是纯新增)。
///
/// pet 窗收到关闭请求(Alt+F4 等)= 隐藏而非销毁 —— 宠物是常驻装饰,
/// 真正的退出仍然只走主窗关闭(句流「主窗关闭 = 退出」的语义不变)。
pub fn on_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    if window.label() != runtime::PET_WINDOW {
        return;
    }
    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
        api.prevent_close();
        let _ = window.hide();
    }
}

/// 心跳一格(由 [`PetState::start_heartbeat`] 的线程每秒调用)。
fn heartbeat_tick(app: &AppHandle) {
    let state = state_of(app);
    reminder::tick(app, &state.pet.reminders);
}

// ---------------------------------------------------------------- 主题联动

/// 句流外观变更时广播给宠物窗(纯新增事件,对既有前端无影响)。
///
/// 宠物窗的气泡与右键菜单用句流令牌配色,`data-theme` 与主窗同源同值 ——
/// HatchDesk 自己那套 localStorage 主题(以及跨窗 `theme-changed` 广播)整体废弃。
pub fn emit_theme(app: &AppHandle, theme: Theme) {
    let _ = app.emit(EV_THEME, theme);
}

// ---------------------------------------------------------------- 设置写入

/// 写 pet 设置分节:落盘 → 更新内存 → 按差异联动窗口 → 广播 `pet://settings`。
///
/// 锁序:先取 settings 锁读旧值并克隆,**释放后**才调 `save_settings`(要 progress 锁),
/// 全程不同时持有两把锁,也不在持锁时碰窗口 API。
pub(crate) fn apply_settings(
    app: &AppHandle,
    state: &AppState,
    next: PetSettings,
) -> CmdResult<PetSettings> {
    let (prev, merged) = {
        let cur = state.settings.lock().expect("settings lock");
        let prev = cur.pet.clone();
        let mut merged = cur.clone();
        merged.pet = next;
        (prev, merged)
    };
    state.save_settings(&merged)?;
    let saved = merged.pet.clone();
    *state.settings.lock().expect("settings lock") = merged;

    // —— 差异联动 ——
    if prev.enabled != saved.enabled {
        if saved.enabled {
            enable(app, &saved)?;
        } else {
            disable(app);
        }
    }
    if saved.enabled {
        if (prev.scale - saved.scale).abs() > f32::EPSILON {
            runtime::apply_scale(app, saved.scale)?;
        }
        if prev.click_through != saved.click_through {
            runtime::set_click_through(app, saved.click_through)?;
        }
    }
    if prev.watcher_enabled && !saved.watcher_enabled {
        watcher::stop(&state.pet.watcher);
    }

    let _ = app.emit(EV_SETTINGS, saved.clone());
    Ok(saved)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 事件名一律 `pet://` 前缀:与句流既有的 `chat://`、`ask://`、`bench://`
    /// 同一惯例,也是「与现有 17 个事件零碰撞」的机械保证。
    #[test]
    fn every_event_is_namespaced() {
        for ev in [
            EV_CHANGED,
            EV_SETTINGS,
            EV_THEME,
            EV_REMINDER,
            EV_POMODORO,
            EV_EXPORT_PROGRESS,
            EV_VIDEO_PROGRESS,
            EV_WATCHER_FILE,
            EV_NAV,
        ] {
            assert!(ev.starts_with("pet://"), "{ev} 缺少命名空间");
        }
    }

    /// 内核错误转译后 code 固定为 `pet`,前端可据此统一归因;
    /// message 保持内核写好的中文,不再包一层。
    #[test]
    fn kernel_errors_keep_their_chinese_message() {
        let e: CmdError = sf_pet::PetError::msg("请先给宠物起个名字").into();
        assert_eq!(e.code, "pet");
        assert_eq!(e.message, "请先给宠物起个名字");
    }
}
