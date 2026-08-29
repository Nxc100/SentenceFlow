//! AI 萌宠命令面(52 个 `pet_*`)。
//!
//! 每个命令都很薄:取参 → 调 [`sf_pet`] → 落盘 → 广播事件。判断、算法、
//! 文件布局一律在内核里,这里只处理「与窗口/设置/事件打交道」的部分。
//!
//! 命名:全部 `pet_` 前缀 —— 与句流既有 65 个命令零碰撞,且 `settings_get`、
//! `work_area_get` 这类通用名不会在未来撞车。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine as _;
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use sf_pet::exporter::{self, ffmpeg};
use sf_pet::imaging::{layout, to_png_data_url};
use sf_pet::petkit::{PetSpec, StateId};
use sf_pet::{archive, rig, spellbook, store, validator, wizard};

use super::reminder::{PomodoroCfg, PomodoroStatus, Reminder};
use super::runtime::{self, WorkArea};
use super::{EV_CHANGED, EV_EXPORT_PROGRESS, EV_VIDEO_PROGRESS, watcher};
use crate::error::{CmdError, CmdResult};
use crate::settings::{PetSettings, Theme};
use crate::state::AppState;

type S<'a> = State<'a, Arc<AppState>>;

fn pets_root(state: &AppState) -> PathBuf {
    state.pet.paths.pets_dir()
}

/// 当前出场宠物 ID(读一次设置就放锁)。
fn active_pet(state: &AppState) -> Option<String> {
    state
        .settings
        .lock()
        .expect("settings lock")
        .pet
        .active_pet
        .clone()
}

fn pet_settings(state: &AppState) -> PetSettings {
    state.settings.lock().expect("settings lock").pet.clone()
}

fn emit_changed(app: &AppHandle, pet_id: Option<&str>, hatch: bool, evolved: Option<Vec<String>>) {
    let _ = app.emit(
        EV_CHANGED,
        serde_json::json!({ "petId": pet_id, "hatch": hatch, "evolved": evolved }),
    );
}

// ---------------------------------------------------------------- 启动包 / 设置

/// 宠物窗启动包:一次拿齐设置、主题与当前图集,省掉三次往返。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PetBootstrap {
    pub settings: PetSettings,
    /// 句流当前主题(宠物窗据此写 `data-theme`,与主窗同源同值)。
    pub theme: Theme,
    pub active: Option<ActivePetAssets>,
}

#[tauri::command]
pub fn pet_bootstrap(state: S<'_>) -> CmdResult<PetBootstrap> {
    let (settings, theme) = {
        let s = state.settings.lock().expect("settings lock");
        (s.pet.clone(), s.appearance.theme)
    };
    let active = load_active_assets(&pets_root(&state), settings.active_pet.as_deref())?;
    Ok(PetBootstrap {
        settings,
        theme,
        active,
    })
}

#[tauri::command]
pub fn pet_settings_get(state: S<'_>) -> PetSettings {
    pet_settings(&state)
}

#[tauri::command]
pub fn pet_settings_set(
    app: AppHandle,
    state: S<'_>,
    settings: PetSettings,
) -> CmdResult<PetSettings> {
    super::apply_settings(&app, &state, settings)
}

// ---------------------------------------------------------------- 宠物库

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivePetAssets {
    pub pet_id: String,
    pub spec: PetSpec,
    /// data URL(webp/png)
    pub sheet: String,
}

fn load_active_assets(root: &Path, pet_id: Option<&str>) -> CmdResult<Option<ActivePetAssets>> {
    let Some(pet_id) = pet_id else {
        return Ok(None);
    };
    // 宠物目录被外部删掉时优雅降级为「还没有宠物」,而不是启动即报错
    let Ok(spec) = store::load_spec(root, pet_id) else {
        return Ok(None);
    };
    let (bytes, mime) = store::load_sheet_bytes(root, pet_id)?;
    let sheet = format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    );
    Ok(Some(ActivePetAssets {
        pet_id: pet_id.to_string(),
        spec,
        sheet,
    }))
}

#[tauri::command]
pub fn pet_list(state: S<'_>) -> CmdResult<Vec<store::PetMeta>> {
    Ok(store::list_pets(&pets_root(&state))?)
}

#[tauri::command]
pub fn pet_thumb(state: S<'_>, pet_id: String) -> CmdResult<String> {
    let root = pets_root(&state);
    let spec = store::load_spec(&root, &pet_id)?;
    let sheet = store::load_sheet(&root, &pet_id)?;
    let row = spec.clip(StateId::Idle).map(|c| c.row).unwrap_or(0);
    Ok(to_png_data_url(&layout::cell_at(&sheet, row, 0))?)
}

#[tauri::command]
pub fn pet_active_assets(state: S<'_>) -> CmdResult<Option<ActivePetAssets>> {
    load_active_assets(&pets_root(&state), active_pet(&state).as_deref())
}

#[tauri::command]
pub fn pet_set_active(app: AppHandle, state: S<'_>, pet_id: String) -> CmdResult<()> {
    store::load_spec(&pets_root(&state), &pet_id)?; // 校验存在
    let mut next = pet_settings(&state);
    next.active_pet = Some(pet_id.clone());
    super::apply_settings(&app, &state, next)?;
    emit_changed(&app, Some(&pet_id), false, None);
    Ok(())
}

/// 认主仪式:写入校准锚点(眼×2/嘴/尾尖,单元格坐标系)到 pet.json,
/// 广播 `pet://changed` 令运行时重载 —— 解锁本地眨眼(双渲染)、精准嘴锚、尾摆。
/// 向后兼容合并:只覆盖传进来的那几个锚点。
#[tauri::command]
pub fn pet_set_anchors(
    app: AppHandle,
    state: S<'_>,
    pet_id: String,
    anchors: BTreeMap<String, sf_pet::petkit::Anchor>,
) -> CmdResult<()> {
    let root = pets_root(&state);
    let mut spec = store::load_spec(&root, &pet_id)?;
    let map = spec.anchors.get_or_insert_with(BTreeMap::new);
    for (k, v) in anchors {
        map.insert(k, v);
    }
    store::save_spec(&root, &pet_id, &spec)?;
    emit_changed(&app, Some(&pet_id), false, None);
    Ok(())
}

#[tauri::command]
pub fn pet_delete(app: AppHandle, state: S<'_>, pet_id: String) -> CmdResult<()> {
    store::delete_pet(&pets_root(&state), &pet_id)?;
    if active_pet(&state).as_deref() == Some(pet_id.as_str()) {
        let mut next = pet_settings(&state);
        next.active_pet = None;
        super::apply_settings(&app, &state, next)?;
        emit_changed(&app, None, false, None);
    }
    Ok(())
}

/// 本地零生成动画引擎·骨骼烘焙:取 idle 首帧 + 关节(认主或启发式)→
/// 轮廓→网格→ARAP→侧视步态→软光栅→烘焙,原地进化为「会走会跳会呼吸」的图集。
/// 全程本地、不经任何平台、零生成式 AI。
///
/// **必须异步**:烘焙是数百毫秒到数秒的纯计算,同步跑会占住 IPC 线程,
/// 连带卡住句流的所有命令(造句工坊正在生成时尤其明显)。
#[tauri::command]
pub async fn pet_rig_bake(app: AppHandle, state: S<'_>, pet_id: String) -> CmdResult<()> {
    let root = pets_root(&state);
    let id = pet_id.clone();
    tauri::async_runtime::spawn_blocking(move || bake_blocking(&root, &id))
        .await
        .map_err(|e| CmdError::new("pet", format!("骨骼烘焙线程异常: {e}")))??;
    let _ = app.emit(
        EV_CHANGED,
        serde_json::json!({
            "petId": pet_id,
            "hatch": false,
            "evolved": ["会走路", "会跳", "会呼吸"],
        }),
    );
    Ok(())
}

fn bake_blocking(root: &Path, pet_id: &str) -> CmdResult<()> {
    let spec = store::load_spec(root, pet_id)?;

    // 源图取「原始 idle 帧」的持久备份:重烘幂等、不在已烘焙帧上再烘(避免累积劣化)、可恢复。
    let rig_src = store::rig_source_path(root, pet_id)?;
    let source = if rig_src.exists() {
        image::open(&rig_src)
            .map_err(|e| CmdError::new("pet", format!("读取烘焙源图失败: {e}")))?
            .to_rgba8()
    } else {
        let sheet = store::load_sheet(root, pet_id)?;
        let idle_row = spec.clip(StateId::Idle).map(|c| c.row).unwrap_or(0);
        let cell = layout::cell_at(&sheet, idle_row, 0);
        std::fs::write(&rig_src, sf_pet::imaging::encode_png(&cell)?)?;
        cell
    };

    let baked = rig::bake::bake(&source, &rig_joints(&spec, &source))?;
    let mut states = BTreeMap::new();
    for bs in &baked.states {
        states.insert(
            bs.id.key().to_string(),
            sf_pet::petkit::StateClip {
                row: bs.id.row(),
                frames: bs.frames,
                fps: bs.fps,
                looping: bs.looping,
                mirror_of: bs.mirror_of.clone(),
                // 烘焙产物:walk 是把 idle 帧扭出来的,运行时仍交给 cutout 分层剪纸
                source: Some(sf_pet::petkit::SOURCE_BAKED.into()),
            },
        );
    }
    let mut new_spec = spec.clone();
    new_spec.states = states;
    new_spec.tier = Some("L2".into());
    store::overwrite_pet(root, pet_id, &new_spec, &baked.sheet)?;
    Ok(())
}

/// 组装 8 关节:`spec.anchors` 里的同名关节优先,缺省走包围盒启发式。
fn rig_joints(spec: &PetSpec, source: &RgbaImage) -> rig::Joints {
    let mut j = match layout::bbox(source) {
        Some(b) => rig::Joints::heuristic(b.x0 as f64, b.y0 as f64, b.w as f64, b.h as f64),
        None => rig::Joints::heuristic(0.0, 0.0, 192.0, 208.0),
    };
    let Some(a) = &spec.anchors else { return j };
    let get = |name: &str| a.get(name).map(|p| rig::V2::new(p.x as f64, p.y as f64));
    if let Some(v) = get("head") {
        j.head = v;
    }
    if let Some(v) = get("neck") {
        j.neck = v;
    }
    if let Some(v) = get("shoulder_l") {
        j.shoulder_l = v;
    }
    if let Some(v) = get("shoulder_r") {
        j.shoulder_r = v;
    }
    if let Some(v) = get("hip") {
        j.hip = v;
    }
    if let Some(v) = get("knee_l") {
        j.knee_l = v;
    }
    if let Some(v) = get("knee_r") {
        j.knee_r = v;
    }
    if let Some(v) = get("ankle") {
        j.ankle = v;
    }
    j
}

// ---------------------------------------------------------------- .petkit 宠物包

#[tauri::command]
pub fn pet_kit_export(state: S<'_>, pet_id: String, dest: String) -> CmdResult<()> {
    Ok(archive::export_petkit(
        &pets_root(&state),
        &pet_id,
        Path::new(&dest),
    )?)
}

#[tauri::command]
pub fn pet_kit_validate(src: String) -> CmdResult<validator::Report> {
    Ok(archive::validate_petkit_file(Path::new(&src))?)
}

#[tauri::command]
pub fn pet_kit_import(app: AppHandle, state: S<'_>, src: String) -> CmdResult<String> {
    let pet_id = archive::import_petkit(&pets_root(&state), state.pet.next_seq(), Path::new(&src))?;
    activate_hatched(&app, &state, &pet_id)?;
    Ok(pet_id)
}

/// 新宠物落地后的收尾:设为当前 + 记下「已完成首次孵化」+ 广播 `pet://changed`。
fn activate_hatched(app: &AppHandle, state: &AppState, pet_id: &str) -> CmdResult<()> {
    let mut next = pet_settings(state);
    next.active_pet = Some(pet_id.to_string());
    next.first_hatch_done = true;
    super::apply_settings(app, state, next)?;
    emit_changed(app, Some(pet_id), true, None);
    Ok(())
}

// ---------------------------------------------------------------- 合成向导

#[tauri::command]
pub fn pet_wizard_reset(state: S<'_>) {
    state
        .pet
        .wizard
        .0
        .lock()
        .expect("pet wizard lock")
        .strips
        .clear();
}

#[tauri::command]
pub fn pet_wizard_session(state: S<'_>) -> CmdResult<wizard::SessionView> {
    Ok(wizard::session_view(
        &state.pet.wizard.0.lock().expect("pet wizard lock"),
    )?)
}

#[tauri::command]
pub fn pet_wizard_add_strip(
    state: S<'_>,
    state_key: String,
    path: String,
    tolerance: Option<f32>,
    forced_frames: Option<u32>,
) -> CmdResult<wizard::SessionView> {
    require_state_key(&state_key)?;
    // 抠图+切帧在锁外做,临界区只覆盖一次插入
    let data = wizard::process_image(&path, tolerance, forced_frames, false)?;
    let mut session = state.pet.wizard.0.lock().expect("pet wizard lock");
    session.strips.insert(state_key, data);
    Ok(wizard::session_view(&session)?)
}

/// 网格整图导入:一次拖入点亮多个状态。
#[tauri::command]
pub fn pet_wizard_add_sheet(
    state: S<'_>,
    path: String,
    states: Vec<String>,
    cols: Option<u32>,
) -> CmdResult<wizard::SessionView> {
    let rois = spellbook::watermark_rois();
    let mut session = state.pet.wizard.0.lock().expect("pet wizard lock");
    wizard::add_sheet(&mut session, &path, &states, cols, &rois)?;
    Ok(wizard::session_view(&session)?)
}

/// 视频处理进度(抠像是秒级操作,前端必须看得见)。
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoProgress {
    pub stage: String,
    pub done: usize,
    pub total: usize,
}

/// 构造一个把进度打到前端的回调(可能来自任意工作线程,故只捕获 `AppHandle`)。
fn video_progress(app: &AppHandle) -> impl Fn(&str, usize, usize) + Sync + '_ {
    move |stage: &str, done: usize, total: usize| {
        let _ = app.emit(
            EV_VIDEO_PROGRESS,
            VideoProgress {
                stage: stage.to_string(),
                done,
                total,
            },
        );
    }
}

/// 视频抽帧导入:处理在阻塞线程执行,不卡 IPC。
#[tauri::command]
pub async fn pet_wizard_add_video(
    app: AppHandle,
    state: S<'_>,
    path: String,
    state_key: String,
    frames: Option<u32>,
) -> CmdResult<wizard::SessionView> {
    require_state_key(&state_key)?;
    let ffmpeg_override = pet_settings(&state).ffmpeg_path;
    let target = frames.unwrap_or(6);
    let app2 = app.clone();
    let strip = tauri::async_runtime::spawn_blocking(move || {
        let progress = video_progress(&app2);
        wizard::process_video(
            ffmpeg_override.as_deref(),
            &path,
            target,
            &spellbook::watermark_rois(),
            &progress,
        )
    })
    .await
    .map_err(|e| CmdError::new("pet", format!("视频处理线程异常: {e}")))??;

    let mut session = state.pet.wizard.0.lock().expect("pet wizard lock");
    session.strips.insert(state_key, strip);
    Ok(wizard::session_view(&session)?)
}

/// 一段视频里的一个动作片段(前端传入的分段契约)。
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SegmentSpec {
    pub state_key: String,
    pub start: f32,
    pub end: f32,
    pub frames: Option<u32>,
}

/// 一次生成、多状态点亮:一段按脚本演完多个动作的视频 → 逐段成为一条状态动画。
#[tauri::command]
pub async fn pet_wizard_add_video_multi(
    app: AppHandle,
    state: S<'_>,
    path: String,
    segments: Vec<SegmentSpec>,
) -> CmdResult<wizard::SessionView> {
    if segments.is_empty() {
        return Err(CmdError::new("pet", "没有指定任何动作片段"));
    }
    let ffmpeg_override = pet_settings(&state).ffmpeg_path;
    let segs: Vec<wizard::VideoSegment> = segments
        .into_iter()
        .map(|s| wizard::VideoSegment {
            state_key: s.state_key,
            start: s.start,
            end: s.end,
            target_frames: s.frames.unwrap_or(6),
        })
        .collect();

    let app2 = app.clone();
    let strips = tauri::async_runtime::spawn_blocking(move || {
        let progress = video_progress(&app2);
        wizard::process_video_segments(
            ffmpeg_override.as_deref(),
            &path,
            &segs,
            &spellbook::watermark_rois(),
            &progress,
        )
    })
    .await
    .map_err(|e| CmdError::new("pet", format!("视频处理线程异常: {e}")))??;

    let mut session = state.pet.wizard.0.lock().expect("pet wizard lock");
    for (key, strip) in strips {
        session.strips.insert(key, strip);
    }
    Ok(wizard::session_view(&session)?)
}

/// 行序调换(网格行序颠倒时一键交换)。
#[tauri::command]
pub fn pet_wizard_swap_states(
    state: S<'_>,
    a: String,
    b: String,
) -> CmdResult<wizard::SessionView> {
    let mut session = state.pet.wizard.0.lock().expect("pet wizard lock");
    wizard::swap_states(&mut session, &a, &b)?;
    Ok(wizard::session_view(&session)?)
}

#[tauri::command]
pub fn pet_wizard_remove_strip(state: S<'_>, state_key: String) -> CmdResult<wizard::SessionView> {
    let mut session = state.pet.wizard.0.lock().expect("pet wizard lock");
    session.strips.remove(&state_key);
    Ok(wizard::session_view(&session)?)
}

#[tauri::command]
pub fn pet_wizard_preview(state: S<'_>) -> CmdResult<wizard::ComposeView> {
    Ok(wizard::preview(
        &state.pet.wizard.0.lock().expect("pet wizard lock"),
    )?)
}

#[tauri::command]
pub fn pet_wizard_hatch(
    app: AppHandle,
    state: S<'_>,
    name: String,
) -> CmdResult<wizard::HatchResult> {
    let root = pets_root(&state);
    let seq = state.pet.next_seq();
    let result = {
        let session = state.pet.wizard.0.lock().expect("pet wizard lock");
        wizard::hatch(&root, seq, &session, &name)?
    };
    if let (true, Some(pet_id)) = (result.ok, result.pet_id.as_deref()) {
        state
            .pet
            .wizard
            .0
            .lock()
            .expect("pet wizard lock")
            .strips
            .clear();
        activate_hatched(&app, &state, pet_id)?;
    }
    Ok(result)
}

/// 进化:把会话素材增量合并进当前宠物。
#[tauri::command]
pub fn pet_wizard_evolve(app: AppHandle, state: S<'_>) -> CmdResult<wizard::EvolveResult> {
    let pet_id = active_pet(&state)
        .ok_or_else(|| CmdError::new("pet", "还没有宠物:先拖入一张图完成首次孵化"))?;
    let root = pets_root(&state);
    let result = {
        let session = state.pet.wizard.0.lock().expect("pet wizard lock");
        wizard::evolve(&root, &pet_id, &session)?
    };
    if result.ok {
        state
            .pet
            .wizard
            .0
            .lock()
            .expect("pet wizard lock")
            .strips
            .clear();
        emit_changed(&app, Some(&pet_id), false, Some(result.unlocked.clone()));
    }
    Ok(result)
}

#[tauri::command]
pub fn pet_wizard_reslice(
    state: S<'_>,
    state_key: String,
    forced_frames: u32,
) -> CmdResult<wizard::SessionView> {
    let mut session = state.pet.wizard.0.lock().expect("pet wizard lock");
    wizard::reslice(&mut session, &state_key, forced_frames)?;
    Ok(wizard::session_view(&session)?)
}

#[tauri::command]
pub fn pet_wizard_replace_frame(
    state: S<'_>,
    state_key: String,
    frame_idx: usize,
    path: String,
    tolerance: Option<f32>,
) -> CmdResult<wizard::SessionView> {
    let mut session = state.pet.wizard.0.lock().expect("pet wizard lock");
    wizard::replace_frame(&mut session, &state_key, frame_idx, &path, tolerance)?;
    Ok(wizard::session_view(&session)?)
}

#[tauri::command]
pub fn pet_wizard_export_frame(
    state: S<'_>,
    state_key: String,
    frame_idx: usize,
    dest: String,
) -> CmdResult<()> {
    let session = state.pet.wizard.0.lock().expect("pet wizard lock");
    Ok(wizard::export_frame(
        &session, &state_key, frame_idx, &dest,
    )?)
}

#[tauri::command]
pub fn pet_l0_preview(path: String, tolerance: Option<f32>) -> CmdResult<serde_json::Value> {
    Ok(wizard::l0_preview(&path, tolerance)?)
}

/// L0 孵化:单图 → idle 单帧宠物(≤60s 上桌面的那条路)。
#[tauri::command]
pub fn pet_l0_hatch(
    app: AppHandle,
    state: S<'_>,
    path: String,
    tolerance: Option<f32>,
    name: String,
) -> CmdResult<wizard::HatchResult> {
    let result = wizard::l0_hatch(
        &pets_root(&state),
        state.pet.next_seq(),
        &path,
        tolerance,
        &name,
    )?;
    if let (true, Some(pet_id)) = (result.ok, result.pet_id.as_deref()) {
        activate_hatched(&app, &state, pet_id)?;
    }
    Ok(result)
}

/// 参考图导出:idle 首帧合成到白底 PNG。
///
/// `dest` 为空时弹「另存为」,默认文件名由宠物名净化而来
/// (原实现硬编码写进 `~/HatchDesk/`,句流改由用户选位置)。
#[tauri::command]
pub async fn pet_reference_export(
    state: S<'_>,
    pet_id: Option<String>,
    dest: Option<String>,
) -> CmdResult<String> {
    let root = pets_root(&state);
    let id = match pet_id {
        Some(id) => id,
        None => active_pet(&state).ok_or_else(|| CmdError::new("pet", "还没有当前宠物"))?,
    };
    let dest = match dest {
        Some(d) => PathBuf::from(d),
        None => {
            let name = store::load_spec(&root, &id)?.name;
            let picked = rfd::AsyncFileDialog::new()
                .set_title("保存参考图")
                .set_file_name(wizard::reference_filename(&name))
                .add_filter("PNG 图片", &["png"])
                .save_file()
                .await;
            match picked {
                Some(h) => h.path().to_path_buf(),
                None => return Err(CmdError::new("pet", "已取消")),
            }
        }
    };
    wizard::export_reference(&root, &id, &dest)?;
    Ok(dest.to_string_lossy().to_string())
}

fn require_state_key(key: &str) -> CmdResult<StateId> {
    StateId::from_key(key).ok_or_else(|| CmdError::new("pet", format!("未知状态 {key}")))
}

// ---------------------------------------------------------------- 下载监听

#[tauri::command]
pub fn pet_watcher_start(app: AppHandle, state: S<'_>) -> CmdResult<()> {
    if !pet_settings(&state).watcher_enabled {
        return Err(CmdError::new("pet", "下载监听已在「AI 萌宠 · 设置」中关闭"));
    }
    watcher::start(&app, &state.pet.watcher)
}

#[tauri::command]
pub fn pet_watcher_stop(state: S<'_>) {
    watcher::stop(&state.pet.watcher);
}

// ---------------------------------------------------------------- 咒语包

#[tauri::command]
pub fn pet_spellbook_get() -> spellbook::SpellbookBundle {
    spellbook::bundle()
}

#[tauri::command]
pub fn pet_guide_save(frames: u32, rows: Option<u32>, dest: String) -> CmdResult<()> {
    let img = spellbook::guide::generate_grid_guide(rows.unwrap_or(1), frames);
    std::fs::write(dest, sf_pet::imaging::encode_png(&img)?)?;
    Ok(())
}

#[tauri::command]
pub fn pet_rescue_get(state_key: String, frames: Option<u32>) -> String {
    spellbook::rescue_spell(&state_key, frames)
}

// ---------------------------------------------------------------- 提醒 / 番茄钟

#[tauri::command]
pub fn pet_reminders_list(state: S<'_>) -> Vec<Reminder> {
    state.pet.reminders.list()
}

#[tauri::command]
pub fn pet_reminder_create(
    state: S<'_>,
    title: String,
    kind: String,
    at: Option<String>,
    minutes: Option<u32>,
) -> CmdResult<Reminder> {
    state.pet.reminders.create(title, kind, at, minutes)
}

#[tauri::command]
pub fn pet_reminder_toggle(state: S<'_>, id: String, enabled: bool) -> CmdResult<()> {
    state.pet.reminders.toggle(&id, enabled)
}

#[tauri::command]
pub fn pet_reminder_delete(state: S<'_>, id: String) -> CmdResult<()> {
    state.pet.reminders.delete(&id)
}

#[tauri::command]
pub fn pet_reminder_snooze(state: S<'_>, title: String, reminder_id: Option<String>, minutes: u32) {
    state.pet.reminders.snooze(title, reminder_id, minutes);
}

#[tauri::command]
pub fn pet_pomodoro_start(state: S<'_>, cfg: PomodoroCfg) -> PomodoroStatus {
    state.pet.reminders.pomodoro_start(cfg)
}

#[tauri::command]
pub fn pet_pomodoro_stop(state: S<'_>) -> PomodoroStatus {
    state.pet.reminders.pomodoro_stop()
}

#[tauri::command]
pub fn pet_pomodoro_status(state: S<'_>) -> PomodoroStatus {
    state.pet.reminders.pomodoro_status()
}

// ---------------------------------------------------------------- 出生视频

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExporterStatus {
    pub found: bool,
    pub path: Option<String>,
}

#[tauri::command]
pub fn pet_exporter_status(state: S<'_>) -> ExporterStatus {
    let override_path = pet_settings(&state).ffmpeg_path;
    match ffmpeg::locate(override_path.as_deref()) {
        Some(p) if ffmpeg::probe(&p) => ExporterStatus {
            found: true,
            path: Some(p.to_string_lossy().to_string()),
        },
        _ => ExporterStatus {
            found: false,
            path: None,
        },
    }
}

fn decode_b64_image(data: Option<String>) -> CmdResult<Option<RgbaImage>> {
    let Some(raw) = data else { return Ok(None) };
    let b64 = raw.rsplit("base64,").next().unwrap_or(&raw);
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .map_err(|e| CmdError::new("pet", format!("图片数据解码失败: {e}")))?;
    Ok(Some(
        image::load_from_memory(&bytes)
            .map_err(|e| CmdError::new("pet", format!("图片数据解码失败: {e}")))?
            .to_rgba8(),
    ))
}

/// 出生视频:15s 竖版 720×1280(孵蛋→破壳→走秀→落版),ffmpeg 管道编码。
#[tauri::command]
pub async fn pet_export_birth_video(
    app: AppHandle,
    state: S<'_>,
    pet_id: String,
    dest: String,
    name_card_png: Option<String>,
    watermark_png: Option<String>,
    ai_notice_png: Option<String>,
) -> CmdResult<()> {
    let root = pets_root(&state);
    let ffmpeg_override = pet_settings(&state).ffmpeg_path;
    let ffmpeg_path = ffmpeg::locate(ffmpeg_override.as_deref()).ok_or_else(|| {
        CmdError::new(
            "pet",
            "未找到 ffmpeg:请先安装 ffmpeg,或在「AI 萌宠 · 设置」中指定其路径",
        )
    })?;

    let spec = store::load_spec(&root, &pet_id)?;
    let sheet = store::load_sheet(&root, &pet_id)?;
    let name_card = decode_b64_image(name_card_png)?;
    let watermark = decode_b64_image(watermark_png)?;
    let ai_notice = decode_b64_image(ai_notice_png)?;

    let progress_app = app.clone();
    let dest_path = PathBuf::from(&dest);
    tauri::async_runtime::spawn_blocking(move || -> CmdResult<()> {
        let mut enc = ffmpeg::Encoder::start(
            &ffmpeg_path,
            exporter::VID_W,
            exporter::VID_H,
            exporter::FPS,
            &dest_path,
        )?;
        let job = exporter::BirthVideoJob {
            spec: &spec,
            sheet: &sheet,
            name_card,
            watermark,
            ai_notice,
        };
        exporter::render_birth_video(&job, |frame, i, total| {
            enc.write_frame(frame.as_raw())?;
            if i % 15 == 0 || i + 1 == total {
                let _ = progress_app.emit(
                    EV_EXPORT_PROGRESS,
                    serde_json::json!({ "done": i + 1, "total": total }),
                );
            }
            Ok(())
        })?;
        enc.finish()?;
        Ok(())
    })
    .await
    .map_err(|e| CmdError::new("pet", format!("视频导出线程异常: {e}")))??;
    Ok(())
}

// ---------------------------------------------------------------- 运行时

/// 宠物窗所在显示器的工作区。
///
/// 原实现按「调用命令的那个窗」注入,主窗调用会拿到主窗的显示器 —— 改成显式取
/// pet 窗,消除这个静默误用面。
#[tauri::command]
pub fn pet_work_area_get(app: AppHandle) -> CmdResult<WorkArea> {
    runtime::work_area(&app)
}

/// 全局光标物理坐标(宠物窗逐像素命中测试用:穿透状态下 webview 收不到鼠标事件,
/// 只能由前端轮询此命令判断光标是否回到了宠物轮廓上)。
#[tauri::command]
pub fn pet_cursor_position_get(app: AppHandle) -> CmdResult<(f64, f64)> {
    let pos = app
        .cursor_position()
        .map_err(|e| CmdError::new("pet", e.to_string()))?;
    Ok((pos.x, pos.y))
}

#[tauri::command]
pub fn pet_visible_set(app: AppHandle, visible: bool) -> CmdResult<()> {
    runtime::set_visible(&app, visible)
}

#[tauri::command]
pub fn pet_click_through_set(app: AppHandle, state: S<'_>, enabled: bool) -> CmdResult<()> {
    let mut next = pet_settings(&state);
    next.click_through = enabled;
    super::apply_settings(&app, &state, next)?;
    Ok(())
}

/// 「打开孵化器」:主窗聚焦 + 页内切签(原 studio 独立窗已并入主窗)。
#[tauri::command]
pub fn pet_open_studio(app: AppHandle, tab: Option<String>) -> CmdResult<()> {
    runtime::open_studio(&app, tab)
}

// ---------------------------------------------------------------- 文件对话框
// 走 rfd(句流惯例,见 commands::pick_file / chat::pick_folder),不引入 dialog 插件。

#[tauri::command]
pub async fn pet_pick_file(
    title: String,
    filter_name: String,
    extensions: Vec<String>,
) -> CmdResult<Option<String>> {
    let mut dialog = rfd::AsyncFileDialog::new().set_title(&title);
    if !extensions.is_empty() {
        let exts: Vec<&str> = extensions.iter().map(String::as_str).collect();
        dialog = dialog.add_filter(&filter_name, &exts);
    }
    Ok(dialog
        .pick_file()
        .await
        .map(|h| h.path().to_string_lossy().into_owned()))
}

#[tauri::command]
pub async fn pet_save_path(
    title: String,
    default_name: String,
    filter_name: String,
    extensions: Vec<String>,
) -> CmdResult<Option<String>> {
    let mut dialog = rfd::AsyncFileDialog::new()
        .set_title(&title)
        .set_file_name(&default_name);
    if !extensions.is_empty() {
        let exts: Vec<&str> = extensions.iter().map(String::as_str).collect();
        dialog = dialog.add_filter(&filter_name, &exts);
    }
    Ok(dialog
        .save_file()
        .await
        .map(|h| h.path().to_string_lossy().into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// data URL 与裸 base64 都要能吃进来(名片/水印由前端 canvas 生成,
    /// 两种形态在不同浏览器实现里都出现过)。
    #[test]
    fn decodes_both_data_urls_and_bare_base64() {
        let png = sf_pet::imaging::encode_png(&RgbaImage::new(2, 2)).unwrap();
        let b64 = base64::engine::general_purpose::STANDARD.encode(&png);

        assert!(decode_b64_image(None).unwrap().is_none());
        assert_eq!(
            decode_b64_image(Some(b64.clone()))
                .unwrap()
                .unwrap()
                .dimensions(),
            (2, 2)
        );
        assert_eq!(
            decode_b64_image(Some(format!("data:image/png;base64,{b64}")))
                .unwrap()
                .unwrap()
                .dimensions(),
            (2, 2)
        );
        assert!(decode_b64_image(Some("不是图片".into())).is_err());
    }

    /// 未知状态名必须在入口挡下,而不是让它带着走到图集合成里去。
    #[test]
    fn unknown_state_keys_are_rejected_at_the_door() {
        assert_eq!(require_state_key("idle").unwrap(), StateId::Idle);
        assert_eq!(require_state_key("walk-left").unwrap(), StateId::WalkLeft);
        let err = require_state_key("dance").unwrap_err();
        assert_eq!(err.code, "pet");
        assert!(err.message.contains("dance"));
    }
}
