//! 宠物窗运行时:建窗、落位、缩放、显隐、点击穿透。
//!
//! 与 HatchDesk 的差异:
//! * 只剩 **pet** 一个运行时窗口 —— 原 studio 窗并入句流主窗的「AI 萌宠」页,
//!   `open_studio` 改为「主窗 show+focus + 广播 `pet://nav` 让页内切签」;
//! * 窗口 label 常量化为 [`PET_WINDOW`],不再散落字面量;
//! * `work_area_of` 显式取 pet 窗(原命令按调用窗注入,主窗调用会拿到主窗的
//!   显示器,是个静默的误用面)。

use serde::Serialize;
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, PhysicalPosition, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

use crate::error::{CmdError, CmdResult};
use crate::settings::PetSettings;

/// 宠物窗 label。主窗是 `"main"`(tauri.conf.json 默认)。
pub const PET_WINDOW: &str = "pet";
/// 主窗 label。
pub const MAIN_WINDOW: &str = "main";

/// 宠物窗逻辑尺寸基准(×`settings.pet.scale`)。
pub const PET_BASE_W: f64 = 300.0;
pub const PET_BASE_H: f64 = 390.0;

/// 缩放钳位区间:再小认不出形象,再大遮挡办公。
pub const SCALE_MIN: f32 = 0.75;
pub const SCALE_MAX: f32 = 2.0;

/// 宠物窗离工作区右边缘的留白(逻辑像素 × 显示器缩放)。
const RIGHT_MARGIN: f64 = 80.0;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkArea {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub scale: f64,
}

/// 钳到可用倍率。倍率来自 JSON,NaN 会一路传成窗口尺寸 NaN,故显式兜回 1.0。
pub fn clamp_scale(scale: f32) -> f64 {
    if scale.is_nan() {
        return 1.0;
    }
    scale.clamp(SCALE_MIN, SCALE_MAX) as f64
}

pub fn window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(PET_WINDOW)
}

/// 建宠物窗(幂等:已存在时只是显示出来)。
///
/// 参数照搬 HatchDesk 实测值:透明 / 无边框 / 无阴影 / 置顶 / 不进任务栏 /
/// 不可缩放 / `accept_first_mouse`(macOS 下未聚焦时首次点击也要生效)。
pub fn ensure_window(app: &AppHandle, settings: &PetSettings) -> CmdResult<WebviewWindow> {
    if let Some(win) = window(app) {
        win.show().map_err(win_err)?;
        return Ok(win);
    }
    let scale = clamp_scale(settings.scale);
    let pet = WebviewWindowBuilder::new(app, PET_WINDOW, WebviewUrl::App("pet.html".into()))
        .title("AI 萌宠")
        .inner_size(PET_BASE_W * scale, PET_BASE_H * scale)
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .accept_first_mouse(true)
        .build()
        .map_err(win_err)?;
    if settings.click_through {
        let _ = pet.set_ignore_cursor_events(true);
    }
    let _ = position_bottom(&pet);
    Ok(pet)
}

/// 销毁宠物窗(总开关关闭时)。不存在即无操作。
pub fn close_window(app: &AppHandle) {
    if let Some(pet) = window(app) {
        let _ = pet.destroy();
    }
}

/// 默认落位:工作区右下角,底边贴工作区底边。
pub fn position_bottom(pet: &WebviewWindow) -> CmdResult<()> {
    let wa = work_area_of(pet)?;
    let size = pet.outer_size().map_err(win_err)?;
    let x = wa.x + wa.w as i32 - size.width as i32 - (RIGHT_MARGIN * wa.scale) as i32;
    let y = wa.y + wa.h as i32 - size.height as i32;
    pet.set_position(PhysicalPosition::new(x, y))
        .map_err(win_err)?;
    Ok(())
}

/// 当前显示器工作区(物理像素;排除任务栏)。
pub fn work_area_of(win: &WebviewWindow) -> CmdResult<WorkArea> {
    let monitor = win
        .current_monitor()
        .map_err(win_err)?
        .or(win.primary_monitor().map_err(win_err)?)
        .ok_or_else(|| CmdError::new("pet", "找不到显示器"))?;
    let rect = monitor.work_area();
    Ok(WorkArea {
        x: rect.position.x,
        y: rect.position.y,
        w: rect.size.width,
        h: rect.size.height,
        scale: monitor.scale_factor(),
    })
}

/// 宠物窗所在显示器的工作区。宠物窗不在时回落主窗,主窗也没有才报错。
pub fn work_area(app: &AppHandle) -> CmdResult<WorkArea> {
    let win = window(app)
        .or_else(|| app.get_webview_window(MAIN_WINDOW))
        .ok_or_else(|| CmdError::new("pet", "窗口尚未就绪"))?;
    work_area_of(&win)
}

pub fn set_click_through(app: &AppHandle, enabled: bool) -> CmdResult<()> {
    if let Some(pet) = window(app) {
        pet.set_ignore_cursor_events(enabled).map_err(win_err)?;
    }
    Ok(())
}

pub fn set_visible(app: &AppHandle, visible: bool) -> CmdResult<()> {
    if let Some(pet) = window(app) {
        if visible {
            pet.show().map_err(win_err)?;
        } else {
            pet.hide().map_err(win_err)?;
        }
    }
    Ok(())
}

pub fn apply_scale(app: &AppHandle, scale: f32) -> CmdResult<()> {
    if let Some(pet) = window(app) {
        let s = clamp_scale(scale);
        pet.set_size(LogicalSize::new(PET_BASE_W * s, PET_BASE_H * s))
            .map_err(win_err)?;
        position_bottom(&pet)?;
    }
    Ok(())
}

/// 「打开孵化器」:主窗显示 + 聚焦,并广播 `pet://nav` 让「AI 萌宠」页切到目标签。
///
/// 原实现是打开独立的 studio 窗;并入主窗后语义等价,少一套跨窗同步。
pub fn open_studio(app: &AppHandle, tab: Option<String>) -> CmdResult<()> {
    if let Some(main) = app.get_webview_window(MAIN_WINDOW) {
        main.show().map_err(win_err)?;
        main.unminimize().ok();
        main.set_focus().map_err(win_err)?;
    }
    let _ = app.emit(super::EV_NAV, tab.unwrap_or_default());
    Ok(())
}

fn win_err(e: tauri::Error) -> CmdError {
    CmdError::new("pet", e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 缩放必须钳在 0.75–2.0:界面滑块之外还有 `.petkit` / 老配置等入口,
    /// 一个离谱的倍率会造出一个占满屏幕、还置顶的窗口。
    #[test]
    fn scale_is_clamped_both_ways() {
        assert_eq!(clamp_scale(0.1), SCALE_MIN as f64);
        assert_eq!(clamp_scale(99.0), SCALE_MAX as f64);
        assert_eq!(clamp_scale(1.25), 1.25);
        // 手改过的 settings 里出现 NaN 时,不能变成尺寸 NaN 的窗口
        assert_eq!(clamp_scale(f32::NAN), 1.0);
        assert_eq!(clamp_scale(f32::INFINITY), SCALE_MAX as f64);
    }

    /// 窗口 label 是 capabilities/pet.json 的 `windows` 白名单键,
    /// 改名等于宠物窗丢掉全部窗口自管理权限。
    #[test]
    fn window_labels_match_capability_whitelist() {
        assert_eq!(PET_WINDOW, "pet");
        assert_eq!(MAIN_WINDOW, "main");
    }
}
