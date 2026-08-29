//! 合成向导:孵化与进化的编排层。
//!
//! 核心模型:**先孵化,后进化**——
//! 拖入一张图立即 L0 孵化;此后 [`WizardSession`] 作为素材暂存区,
//! 每次导入(单条 / 网格整图 / 视频抽帧)通过 [`evolve`] 增量合并进当前宠物:
//! 全部素材源帧持久化([`store::save_source_frames`]),重合成时全量参与统一缩放,
//! 保证任意次进化后各状态比例一致、质量不降。
//!
//! **零 tauri**:落盘位置由 `pets_root: &Path` 传入,长任务进度由 [`Progress`]
//! 回调注入,「设为当前宠物 / 广播事件」等副作用留给胶水层——本模块只负责
//! 「素材进来、图集出去」这一段。

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;

use image::RgbaImage;
use serde::Serialize;

use crate::error::{PetError, PetResult};
use crate::exporter::ffmpeg;
use crate::imaging::{
    chroma::{self, Background, CornerRoi, SourceKind},
    decoder, denoise, layout, matting, motion, phash, slice, to_png_data_url,
};
use crate::petkit::{PetSpec, StateClip, StateId};
use crate::store;
use crate::validator;

const MAX_INPUT_DIM: u32 = 8192;

// ---------------------------------------------------------------- 会话模型

pub struct StripData {
    pub source_path: String,
    pub frames: Vec<RgbaImage>,
    pub background: Background,
    /// 实际采用的容差（自动估计后的值）
    pub tolerance: f32,
    pub warnings: Vec<String>,
}

#[derive(Default)]
pub struct WizardSession {
    /// key = 状态 key（"idle" 等）
    pub strips: BTreeMap<String, StripData>,
}

/// 向导会话的独占容器。胶水层把它放进 `PetState`,临界区只覆盖一次操作。
#[derive(Default)]
pub struct WizardState(pub Mutex<WizardSession>);

fn bg_label(bg: Background) -> &'static str {
    match bg {
        Background::Transparent => "transparent",
        Background::Chroma => "chroma",
        Background::Solid => "solid",
        Background::Busy => "busy",
    }
}

// ---------------------------------------------------------------- 抠背景管线

struct CutResult {
    img: RgbaImage,
    background: Background,
    used_tolerance: f32,
    warnings: Vec<String>,
}

/// 背景移除（M3 容差自动化 + 环节七 matting 主刀）：
/// - `tolerance = None` → `auto_tolerance` 按背景类型/幕色纯度/来源估计；
/// - `Busy` 且允许 matting → u2netp 主刀，泛洪对照，取前景占比更合理者；
/// - 统一做散点降噪与边缘绿边去污（M7）。
fn remove_background(
    img: RgbaImage,
    tolerance: Option<f32>,
    source: SourceKind,
    allow_matting: bool,
) -> CutResult {
    let background = chroma::detect_background(&img);
    let used_tolerance = tolerance.unwrap_or_else(|| chroma::auto_tolerance(&img, source));
    let mut warnings = Vec::new();

    let mut cut = match background {
        Background::Transparent => {
            let mut i = img;
            chroma::normalize_transparent(&mut i);
            i
        }
        Background::Chroma => chroma::key_out_green(&img, used_tolerance),
        Background::Solid => {
            let mut cut = chroma::flood_remove_background(&img, used_tolerance);
            // 浅色主体配浅色背景时，泛洪的颜色距离阈值会直接吃穿主体
            // （实测：主体与背景距离 72，而容差 60 对应的阈值是 124 —— 主体只剩 3%）。
            // 逐级收紧容差重试；仍不合理再交给语义抠像。
            if !matting::fraction_plausible(matting::foreground_fraction(&cut)) {
                for factor in [0.5f32, 0.25] {
                    let tighter = used_tolerance * factor;
                    let retry = chroma::flood_remove_background(&img, tighter);
                    if matting::fraction_plausible(matting::foreground_fraction(&retry)) {
                        warnings.push(format!(
                            "背景与主体颜色接近，已把抠图容差自动收紧到 {tighter:.0}"
                        ));
                        cut = retry;
                        break;
                    }
                }
            }
            if !matting::fraction_plausible(matting::foreground_fraction(&cut))
                && allow_matting
                && matting::is_available()
            {
                if let Ok(m) = matting::cut_out(&img) {
                    if matting::fraction_plausible(matting::foreground_fraction(&m)) {
                        warnings.push("纯色背景与主体过于接近，已改用智能抠图".into());
                        cut = m;
                    }
                }
            }
            cut
        }
        Background::Busy => {
            let flood = chroma::flood_remove_background(&img, used_tolerance);
            if allow_matting && matting::is_available() {
                match matting::cut_out(&img) {
                    Ok(matted) => {
                        let fm = matting::foreground_fraction(&matted);
                        let ff = matting::foreground_fraction(&flood);
                        if matting::fraction_plausible(fm) {
                            matted
                        } else if matting::fraction_plausible(ff) {
                            warnings.push("智能抠图结果异常，已回退为泛洪抠图".into());
                            flood
                        } else if (fm - 0.35).abs() < (ff - 0.35).abs() {
                            matted
                        } else {
                            flood
                        }
                    }
                    Err(_) => {
                        warnings.push("智能抠图不可用，已回退为泛洪抠图".into());
                        flood
                    }
                }
            } else {
                if allow_matting {
                    warnings.push("背景较复杂，自动抠图尽力而为；可在高级里调整容差后重试".into());
                }
                flood
            }
        }
    };
    let min_area = denoise::default_min_area(cut.width(), cut.height());
    denoise::remove_specks(&mut cut, min_area);
    chroma::despill_edges(&mut cut);
    CutResult {
        img: cut,
        background,
        used_tolerance,
        warnings,
    }
}

fn load_input(path: &str) -> PetResult<RgbaImage> {
    let img = image::open(path)
        .map_err(|e| PetError::msg(format!("无法打开图片 {path}: {e}")))?
        .to_rgba8();
    let (w, h) = img.dimensions();
    if w > MAX_INPUT_DIM || h > MAX_INPUT_DIM {
        return Err(PetError::msg(format!(
            "图片过大（{w}×{h}），最大支持 {MAX_INPUT_DIM}px"
        )));
    }
    if w < 16 || h < 16 {
        return Err(PetError::msg("图片过小，无法处理"));
    }
    Ok(img)
}

/// 角标清除（§5.4 第 2 层）+ 压角软性警告。
fn clean_corners(img: &mut RgbaImage, rois: &[CornerRoi], warnings: &mut Vec<String>) {
    if rois.is_empty() {
        return;
    }
    let report = chroma::clear_corner_watermarks(img, rois);
    if report.cleared > 0 {
        warnings.push(format!("已自动清除疑似平台角标（{} 像素）", report.cleared));
    }
    if report.character_touches_corner {
        warnings
            .push("角色距角落过近，疑似与平台角标重叠，建议重新生成该图（角落必须留空）".into());
    }
}

fn drift_warnings(frames: &[RgbaImage], warnings: &mut Vec<String>) {
    let hashes: Vec<u64> = frames.iter().map(phash::phash).collect();
    for i in 1..hashes.len() {
        let d = phash::hamming(hashes[i - 1], hashes[i]);
        if d > phash::DRIFT_THRESHOLD {
            warnings.push(format!(
                "第 {} 帧与上一帧差异过大（pHash 距离 {d}），疑似身份漂移，建议救帧",
                i + 1
            ));
        }
    }
}

/// 单张图 → 抠背景 → 切帧（一维）。`single=true` 时整图作一帧（L0/单帧替换）。
pub fn process_image(
    path: &str,
    tolerance: Option<f32>,
    forced_frames: Option<u32>,
    single: bool,
) -> PetResult<StripData> {
    let img = load_input(path)?;
    let cut = remove_background(img, tolerance, SourceKind::Image, single);
    let mut warnings = cut.warnings;
    let mut keyed = cut.img;
    if !single {
        clean_corners(&mut keyed, &chroma::default_corner_rois(), &mut warnings);
    }

    let (spans, uniform_fallback) = if single {
        (vec![(0u32, keyed.width())], false)
    } else {
        slice::slice_strip(&keyed, forced_frames)
    };
    if uniform_fallback {
        warnings.push(format!(
            "自动切帧结果与指定的 {} 帧不符，已按等宽切分（帧粘连时属正常）",
            forced_frames.unwrap_or(0)
        ));
    }

    let mut frames = slice::cut_frames(&keyed, &spans);
    let before = frames.len();
    frames.retain(|f| layout::bbox(f).is_some());
    if frames.len() < before {
        warnings.push(format!("丢弃了 {} 个空帧", before - frames.len()));
    }
    if frames.is_empty() {
        return Err(PetError::msg(
            "未检测到任何角色内容：请确认背景为纯绿 #00FF00（或透明），或在高级里调整容差",
        ));
    }
    drift_warnings(&frames, &mut warnings);

    Ok(StripData {
        source_path: path.to_string(),
        frames,
        background: cut.background,
        tolerance: cut.used_tolerance,
        warnings,
    })
}

/// 网格整图导入（环节三）：一次拖入 → 多状态同时点亮。
/// `states` 为行→状态契约（如 ["idle","walk-right"]），行序颠倒可用 `swap_states` 调换。
pub fn add_sheet(
    session: &mut WizardSession,
    path: &str,
    states: &[String],
    expected_cols: Option<u32>,
    rois: &[CornerRoi],
) -> PetResult<Vec<String>> {
    if states.is_empty() {
        return Err(PetError::msg("缺少行→状态契约"));
    }
    for key in states {
        StateId::from_key(key).ok_or_else(|| PetError::msg(format!("未知状态 {key}")))?;
    }
    let img = load_input(path)?;
    let cut = remove_background(img, None, SourceKind::Image, false);
    let mut shared_warnings = cut.warnings;
    let mut keyed = cut.img;
    clean_corners(&mut keyed, rois, &mut shared_warnings);

    let (rows, fallback) = slice::slice_grid(&keyed, states.len() as u32, expected_cols);
    if fallback.uniform_rows {
        shared_warnings.push("行检测与版式不符，已按等高切行（行间距不足时属正常）".into());
    }
    if fallback.uniform_cols {
        shared_warnings.push("部分行的帧检测与版式不符，已按等宽切分".into());
    }

    let mut imported = Vec::new();
    for (row_idx, (state_key, mut frames)) in states.iter().zip(rows).enumerate() {
        let before = frames.len();
        frames.retain(|f| layout::bbox(f).is_some());
        let mut warnings = shared_warnings.clone();
        if frames.len() < before {
            warnings.push(format!("丢弃了 {} 个空帧", before - frames.len()));
        }
        if frames.is_empty() {
            return Err(PetError::msg(format!(
                "第 {} 行（{state_key}）没有可用内容：请检查生成图或按失败自查表重试",
                row_idx + 1
            )));
        }
        drift_warnings(&frames, &mut warnings);
        session.strips.insert(
            state_key.clone(),
            StripData {
                source_path: path.to_string(),
                frames,
                background: cut.background,
                tolerance: cut.used_tolerance,
                warnings,
            },
        );
        imported.push(state_key.clone());
    }
    Ok(imported)
}

// ---------------------------------------------------------------- 视频路线

/// 抠像关键帧数：判定背景能否抠干净、并圈出主体 ROI 供运动分析用。
const ROI_KEYFRAMES: usize = 3;
/// 主体 ROI 外扩比例（给走动、摆尾留余量）。
const ROI_PAD: f32 = 0.12;
/// 独立碎块小于主体这个比例即丢弃（实测阴影碎块约 3%，真部件远大于此）。
const MAIN_SUBJECT_MIN_SHARE: f32 = 0.15;

/// 进度播报：(阶段文案, 已完成, 总数)。
pub type Progress<'a> = &'a (dyn Fn(&str, usize, usize) + Sync);

/// 一段已解码视频的处理上下文。
///
/// 关键设计（性能）：AI 抠像是全管线唯一的计算热点（tract 单线程约 2 秒/帧），
/// 所以**运动分析绝不碰抠像**——分析只在原始帧的主体 ROI 上做，零推理成本；
/// 只有最终选中的那几帧才真正抠像。相比"逐帧抠像再分析"，
/// 一段 10 秒素材的推理次数从 100 次降到约 11 次。
struct ClipContext {
    raw: Vec<RgbaImage>,
    background: Background,
    warnings: Vec<String>,
    /// true = 复杂背景，走 u2netp 语义抠像；false = 绿幕/纯色/透明，走廉价抠色
    use_matting: bool,
    /// 廉价路线：所有帧都已抠好（抠色只是像素运算，不值得省）
    cheap_cut: Vec<RgbaImage>,
    /// 抠像路线：主体 ROI（分析用）
    roi: layout::Bbox,
    tolerance: f32,
}

/// 采样若干帧投票决定背景类型：逐帧判定会在一段素材内来回跳
/// （某几帧恰好边框干净就被判成纯色），导致同一段视频用了两套抠法。
fn sampled_background(frames: &[RgbaImage]) -> Background {
    let n = frames.len();
    let picks = 5.min(n);
    let mut votes: BTreeMap<&'static str, (u32, Background)> = BTreeMap::new();
    for k in 0..picks {
        let bg = chroma::detect_background(&frames[k * n / picks]);
        let e = votes.entry(bg_label(bg)).or_insert((0, bg));
        e.0 += 1;
    }
    votes
        .values()
        .max_by_key(|(c, _)| *c)
        .map(|(_, b)| *b)
        .unwrap_or(Background::Busy)
}

fn prepare_clip(
    ffmpeg_override: Option<&str>,
    path: &str,
    progress: Progress<'_>,
) -> PetResult<ClipContext> {
    let ffmpeg_path = ffmpeg::locate(ffmpeg_override)
        .ok_or_else(|| PetError::msg("未找到 ffmpeg：视频路线需要，请安装或在设置中指定路径"))?;

    progress("正在解码视频", 0, 1);
    let raw = decoder::extract_frames(
        &ffmpeg_path,
        std::path::Path::new(path),
        &decoder::ExtractOptions::default(),
    )?;
    if raw.len() < 2 {
        return Err(PetError::msg(
            "视频里只解出 1 帧，请确认这是一段有效的动态视频",
        ));
    }

    let background = sampled_background(&raw);
    let mut warnings: Vec<String> = Vec::new();

    if background != Background::Busy {
        // 绿幕 / 纯色 / 已透明：抠色是纯像素运算，直接全帧处理
        progress("正在抠背景", 0, raw.len());
        let tolerance = chroma::auto_tolerance(&raw[0], SourceKind::Video);
        let cheap_cut: Vec<RgbaImage> = raw
            .iter()
            .enumerate()
            .map(|(i, f)| {
                progress("正在抠背景", i + 1, raw.len());
                remove_background(f.clone(), Some(tolerance), SourceKind::Video, false).img
            })
            .collect();

        // 抽样合理性检查：背景判定或容差踩空时（浅色主体配浅色背景是典型场景），
        // 廉价抠色会把主体吃掉。宁可多花几秒推理，也不能把碎片当成宠物送下去。
        let probe = 3.min(cheap_cut.len());
        let ok = (0..probe).all(|k| {
            matting::fraction_plausible(matting::foreground_fraction(
                &cheap_cut[k * cheap_cut.len() / probe],
            ))
        });
        if ok {
            return Ok(ClipContext {
                raw,
                background,
                warnings,
                use_matting: false,
                cheap_cut,
                roi: layout::Bbox {
                    x0: 0,
                    y0: 0,
                    w: 1,
                    h: 1,
                    centroid_x: 0.0,
                },
                tolerance,
            });
        }
        warnings.push("按纯色/绿幕抠不干净，已自动改用本地智能抠图".into());
    }

    // 复杂背景（豆包/即梦图生视频的常态：真实场景、地面、光影）。
    // 泛洪抠图在这里是灾难性的——背景均色与浅色主体的距离远小于容差阈值，
    // 泛洪会直接穿透主体，只剩下最深的几块碎片。必须走语义抠像。
    if !matting::is_available() {
        return Err(PetError::msg(
            "这段视频是复杂背景（真实场景），需要本地智能抠图模型，但模型加载失败。\
             请改用纯色/绿幕背景的视频，或改走网格图路线",
        ));
    }

    progress("正在识别主体", 0, ROI_KEYFRAMES);
    let key_idx: Vec<usize> = (0..ROI_KEYFRAMES)
        .map(|k| k * (raw.len() - 1) / ROI_KEYFRAMES.max(1))
        .collect();
    let keys: Vec<RgbaImage> = key_idx.iter().map(|&i| raw[i].clone()).collect();
    let key_cuts = matting::cut_out_batch(&keys, &|d, t| progress("正在识别主体", d, t))?;

    // 合理性闸门：抠出来只剩零星碎片 / 整张几乎全留 都说明这段素材抠不动。
    // 旧实现没有这道闸门，于是碎片被当成宠物一路合成进图集 —— 就是形象损坏的直接原因。
    let fracs: Vec<f32> = key_cuts.iter().map(matting::foreground_fraction).collect();
    let mean_frac = fracs.iter().sum::<f32>() / fracs.len().max(1) as f32;
    if !matting::fraction_plausible(mean_frac) {
        return Err(PetError::msg(format!(
            "没能从这段视频里分离出角色（识别到的主体只占画面 {:.0}%）。\
             常见原因：画面里不止一个主体、角色与背景颜色太接近、或镜头一直在动。\
             建议换一段固定镜头、主体单一、背景简单的视频",
            mean_frac * 100.0
        )));
    }

    let roi = matting::union_roi(&key_cuts, ROI_PAD)
        .ok_or_else(|| PetError::msg("没能在视频里定位到角色主体，请换一段主体更清晰的视频"))?;
    warnings.push("复杂背景已用本地智能抠图分离角色（全程离线）".into());

    Ok(ClipContext {
        raw,
        background: Background::Busy,
        warnings,
        use_matting: true,
        cheap_cut: Vec::new(),
        roi,
        tolerance: 0.0,
    })
}

/// 分析用缩略图 + 每帧主体面积（面积仅在廉价路线可得）。
fn analysis_inputs(
    ctx: &ClipContext,
    range: std::ops::Range<usize>,
) -> (Vec<RgbaImage>, Vec<Option<f32>>) {
    if ctx.use_matting {
        let frames: Vec<RgbaImage> = ctx.raw[range.clone()].to_vec();
        (
            motion::thumbs_from_roi(&frames, &ctx.roi),
            vec![None; range.len()],
        )
    } else {
        let frames: Vec<RgbaImage> = ctx.cheap_cut[range.clone()].to_vec();
        let areas = frames
            .iter()
            .map(|f| layout::bbox(f).map(|b| (b.w * b.h) as f32))
            .collect();
        (motion::thumbs_from_cut(&frames), areas)
    }
}

/// 把选中的（全片）帧下标抠成成品帧。
fn cut_selected(
    ctx: &ClipContext,
    indices: &[usize],
    rois: &[CornerRoi],
    warnings: &mut Vec<String>,
    progress: Progress<'_>,
) -> PetResult<Vec<RgbaImage>> {
    let mut frames: Vec<RgbaImage> = if ctx.use_matting {
        // 去重后再推理：正倒放拼接会让同一帧出现多次，没必要抠两遍
        let mut uniq: Vec<usize> = indices.to_vec();
        uniq.sort_unstable();
        uniq.dedup();
        let picks: Vec<RgbaImage> = uniq.iter().map(|&i| ctx.raw[i].clone()).collect();
        let cut = matting::cut_out_batch(&picks, &|d, t| progress("AI 抠像中", d, t))?;
        let by_idx: BTreeMap<usize, RgbaImage> = uniq.into_iter().zip(cut).collect();
        indices.iter().map(|i| by_idx[i].clone()).collect()
    } else {
        indices.iter().map(|&i| ctx.cheap_cut[i].clone()).collect()
    };

    let mut corner_warned = false;
    for img in &mut frames {
        let min_area = denoise::default_min_area(img.width(), img.height());
        denoise::remove_specks(img, min_area);
        if ctx.use_matting {
            // 语义抠像会把地面阴影/反光判成前景，形成与主体分离的半透明碎块，
            // 只在个别帧出现 → 动画上是脚边一闪一闪的脏东西。只保留主体。
            denoise::keep_main_subject(img, MAIN_SUBJECT_MIN_SHARE);
        }
        chroma::despill_edges(img);
        if !rois.is_empty() {
            let report = chroma::clear_corner_watermarks(img, rois);
            if report.character_touches_corner && !corner_warned {
                corner_warned = true;
                warnings.push("部分帧角色距角落过近，疑似与平台角标重叠".into());
            }
        }
    }

    let before = frames.len();
    frames.retain(|f| layout::bbox(f).is_some());
    if frames.len() < before {
        warnings.push(format!("丢弃了 {} 个空帧", before - frames.len()));
    }
    if frames.len() < 2 {
        return Err(PetError::msg(
            "抠像后没剩下足够的角色内容，请换一段主体更清晰的视频",
        ));
    }

    // 出画检测：角色包围盒贴到画面边缘，说明它有一部分在取景框外，
    // 抠出来就是"缺了一块的宠物"（实测某段走路素材里狗的后半身在画面左侧之外，
    // 做成精灵后尾部是一条直边）。这种缺损无法从像素上补回来，只能如实告知用户重出该段。
    let cropped = frames.iter().filter(|f| touches_frame_edge(f)).count();
    if cropped > 0 {
        warnings.push(format!(
            "有 {cropped}/{} 帧的角色贴到了画面边缘（身体有一部分在取景框外），\
             做成动画会缺一块；请重新生成这一段，强调「角色全身始终完整出现在画面内、不要走出画面」",
            frames.len()
        ));
    }
    Ok(frames)
}

/// 角色是否被画面裁掉了一部分（容 1px，避免羽化边误判）。
///
/// **只看左/右/上三边**：主体站在画面底边上是正常取景（脚落在地面），
/// 且入格时本来就按底边基线对齐，不算缺损；而左右或顶部贴边意味着
/// 身体确实有一截在取景框外，抠出来会是一条直边。
fn touches_frame_edge(img: &RgbaImage) -> bool {
    let (w, _) = img.dimensions();
    layout::bbox(img).is_some_and(|b| b.x0 <= 1 || b.y0 <= 1 || b.x0 + b.w >= w.saturating_sub(1))
}

fn selection_warnings(sel: &decoder::FrameSelection, warnings: &mut Vec<String>) {
    if sel.dropped_unstable > 0 {
        warnings.push(format!(
            "已过滤 {} 个主体不稳定帧（镜头推拉/主体时隐时现）",
            sel.dropped_unstable
        ));
    }
    if sel.used_pingpong {
        warnings.push("视频首尾未闭合，已用「正放+倒放」拼接保证循环平滑".into());
    }
    if sel.loop_closed {
        let p = sel.loop_period.unwrap_or(0);
        warnings.push(format!("已定位到一个完整动作周期（{p} 帧），循环无缝"));
    }
}

/// 视频抽帧处理（环节五）：mp4 → 抽帧 → 运动分析选帧 → 只抠选中帧。
/// 纯处理函数（无会话依赖），供异步命令在阻塞线程执行后再入会话。
pub fn process_video(
    ffmpeg_override: Option<&str>,
    path: &str,
    target_frames: u32,
    rois: &[CornerRoi],
    progress: Progress<'_>,
) -> PetResult<StripData> {
    let ctx = prepare_clip(ffmpeg_override, path, progress)?;
    let mut warnings = ctx.warnings.clone();

    progress("正在挑选动画帧", 0, 1);
    let (thumbs, areas) = analysis_inputs(&ctx, 0..ctx.raw.len());
    let sel = decoder::select_from_analysis(&thumbs, &areas, target_frames as usize)?;
    selection_warnings(&sel, &mut warnings);

    let frames = cut_selected(&ctx, &sel.indices, rois, &mut warnings, progress)?;
    Ok(StripData {
        source_path: path.to_string(),
        frames,
        background: ctx.background,
        tolerance: ctx.tolerance,
        warnings,
    })
}

/// 一个动作片段 → 一个状态（一次生成，多状态点亮）。
#[derive(Clone, Debug)]
pub struct VideoSegment {
    pub state_key: String,
    /// 片段起止，占全片时长的比例（0..1）
    pub start: f32,
    pub end: f32,
    pub target_frames: u32,
}

/// 把片段边界吸附到附近的「运动低谷」：AI 视频在切换动作时几乎总有一个短暂的停顿，
/// 对齐到低谷能显著减少把两个动作切在一起的概率。
fn snap_to_valley(steps: &[f32], boundary: usize, search: usize) -> usize {
    if steps.is_empty() {
        return boundary;
    }
    let lo = boundary.saturating_sub(search).min(steps.len() - 1);
    let hi = (boundary + search).min(steps.len() - 1);
    let mut best = boundary.min(steps.len() - 1);
    let mut best_v = f32::MAX;
    for (i, step) in steps.iter().enumerate().take(hi + 1).skip(lo) {
        if *step < best_v {
            best_v = *step;
            best = i;
        }
    }
    best
}

/// 一次生成、多状态点亮（改进方案环节五修订）：
/// 一段按脚本演完待机/行走/开心/睡觉的视频 → 按时间分段 → 每段独立选帧成一条动画。
/// 分段边界会先按声明的时间比例落点，再吸附到附近的运动低谷。
pub fn process_video_segments(
    ffmpeg_override: Option<&str>,
    path: &str,
    segments: &[VideoSegment],
    rois: &[CornerRoi],
    progress: Progress<'_>,
) -> PetResult<Vec<(String, StripData)>> {
    if segments.is_empty() {
        return Err(PetError::msg("没有指定任何动作片段"));
    }
    for s in segments {
        StateId::from_key(&s.state_key)
            .ok_or_else(|| PetError::msg(format!("未知状态 {}", s.state_key)))?;
    }
    let ctx = prepare_clip(ffmpeg_override, path, progress)?;
    let n = ctx.raw.len();

    // 全片运动曲线（缩略图上算，便宜）——用来吸附分段边界
    let (all_thumbs, _) = analysis_inputs(&ctx, 0..n);
    let steps: Vec<f32> = (0..n.saturating_sub(1))
        .map(|i| motion::motion_between(&all_thumbs[i], &all_thumbs[i + 1]))
        .collect();
    let search = ((n as f32 / segments.len() as f32) * 0.15) as usize;

    let mut out: Vec<(String, StripData)> = Vec::new();
    let mut failures: Vec<String> = Vec::new();
    for (k, seg) in segments.iter().enumerate() {
        let raw_lo = ((seg.start.clamp(0.0, 1.0) * n as f32) as usize).min(n.saturating_sub(2));
        let raw_hi = ((seg.end.clamp(0.0, 1.0) * n as f32) as usize).clamp(raw_lo + 2, n);
        // 首段起点与末段终点不吸附（它们就是素材两端）
        let lo = if k == 0 {
            raw_lo
        } else {
            snap_to_valley(&steps, raw_lo, search)
        };
        let hi = if k == segments.len() - 1 {
            raw_hi
        } else {
            snap_to_valley(&steps, raw_hi, search).max(lo + 2).min(n)
        };

        let name = StateId::from_key(&seg.state_key)
            .map(|s| s.display_name())
            .unwrap_or(&seg.state_key);
        progress(&format!("正在处理「{name}」片段"), k, segments.len());

        let (thumbs, areas) = analysis_inputs(&ctx, lo..hi);
        let sel = match decoder::select_from_analysis(&thumbs, &areas, seg.target_frames as usize) {
            Ok(s) => s,
            Err(e) => {
                failures.push(format!("{name}：{e}"));
                continue;
            }
        };
        // 片段内下标 → 全片下标
        let global: Vec<usize> = sel.indices.iter().map(|i| lo + i).collect();
        let mut warnings = ctx.warnings.clone();
        selection_warnings(&sel, &mut warnings);
        match cut_selected(&ctx, &global, rois, &mut warnings, progress) {
            Ok(frames) => out.push((
                seg.state_key.clone(),
                StripData {
                    source_path: path.to_string(),
                    frames,
                    background: ctx.background,
                    tolerance: ctx.tolerance,
                    warnings,
                },
            )),
            Err(e) => failures.push(format!("{name}：{e}")),
        }
    }

    if out.is_empty() {
        return Err(PetError::msg(format!(
            "所有动作片段都没能抽出可用动画：{}",
            failures.join("；")
        )));
    }
    if !failures.is_empty() {
        // 部分成功：把失败的片段作为警告挂在第一条上，前端会展示
        out[0]
            .1
            .warnings
            .push(format!("以下片段没抽到动画：{}", failures.join("；")));
    }
    Ok(out)
}

/// 行序调换（防模型换序，环节三）。
pub fn swap_states(session: &mut WizardSession, a: &str, b: &str) -> PetResult<()> {
    if a == b {
        return Ok(());
    }
    let sa = session.strips.remove(a);
    let sb = session.strips.remove(b);
    if sa.is_none() && sb.is_none() {
        return Err(PetError::msg("两个状态都没有素材，无从调换"));
    }
    if let Some(s) = sa {
        session.strips.insert(b.to_string(), s);
    }
    if let Some(s) = sb {
        session.strips.insert(a.to_string(), s);
    }
    Ok(())
}

// ---------------------------------------------------------------- 视图

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StripView {
    pub state: String,
    pub state_name: String,
    pub frames: u32,
    pub previews: Vec<String>,
    pub warnings: Vec<String>,
    pub background: String,
    pub recommended: (u32, u32),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionView {
    pub strips: Vec<StripView>,
    pub tier: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ComposeView {
    pub sheet: String,
    pub report: validator::Report,
    pub tier: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HatchResult {
    pub ok: bool,
    pub pet_id: Option<String>,
    pub report: validator::Report,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvolveResult {
    pub ok: bool,
    pub pet_id: String,
    pub report: validator::Report,
    /// 本次点亮/更新的状态（中文名，气泡播报用）
    pub unlocked: Vec<String>,
}

fn strip_view(key: &str, data: &StripData) -> PetResult<StripView> {
    let sid = StateId::from_key(key);
    let refs: Vec<&RgbaImage> = data.frames.iter().collect();
    let scale = layout::global_scale(&refs);
    let previews = data
        .frames
        .iter()
        .map(|f| to_png_data_url(&layout::fit_into_cell(f, scale)))
        .collect::<PetResult<Vec<_>>>()?;
    let mut warnings = data.warnings.clone();
    if let Some(sid) = sid {
        let (lo, hi) = sid.recommended_frames();
        let n = data.frames.len() as u32;
        if n < lo || n > hi {
            warnings.push(format!(
                "帧数 {n} 超出建议范围 {lo}–{hi}，仍可使用，但动画节奏可能不理想"
            ));
        }
    }
    Ok(StripView {
        state: key.to_string(),
        state_name: sid
            .map(|s| s.display_name().to_string())
            .unwrap_or_else(|| key.to_string()),
        frames: data.frames.len() as u32,
        previews,
        warnings,
        background: bg_label(data.background).into(),
        recommended: sid.map(|s| s.recommended_frames()).unwrap_or((1, 8)),
    })
}

pub fn session_view(session: &WizardSession) -> PetResult<SessionView> {
    let mut strips = Vec::new();
    for sid in StateId::ALL {
        if let Some(data) = session.strips.get(sid.key()) {
            strips.push(strip_view(sid.key(), data)?);
        }
    }
    Ok(SessionView {
        tier: estimate_tier(session),
        strips,
    })
}

fn estimate_tier(session: &WizardSession) -> String {
    tier_of_states(
        session.strips.keys().map(|k| k.as_str()),
        session
            .strips
            .get("idle")
            .map(|s| s.frames.len())
            .unwrap_or(0),
    )
}

fn tier_of_states<'a>(keys: impl Iterator<Item = &'a str>, idle_frames: usize) -> String {
    let n = keys.count();
    if n >= 6 {
        "L2".into()
    } else if idle_frames >= 2 {
        "L1".into()
    } else {
        "L0".into()
    }
}

// ---------------------------------------------------------------- 合成

/// 从原始帧集合成图集与 spec（纯函数，孵化与进化共用）：
/// 全局统一缩放 → 入格 → 8×8 图集；右走存在而左走缺失 → 自动镜像派生。
pub fn compose_from_frames(
    frames_by_state: &BTreeMap<String, Vec<RgbaImage>>,
) -> PetResult<(PetSpec, RgbaImage)> {
    if !frames_by_state.contains_key("idle") {
        return Err(PetError::msg("至少需要 idle 待机素材"));
    }
    let all_frames: Vec<&RgbaImage> = frames_by_state.values().flatten().collect();
    let scale = layout::global_scale(&all_frames);

    let idle_frames = frames_by_state.get("idle").map(|f| f.len()).unwrap_or(0);
    let mut spec = PetSpec::new(
        "",
        &tier_of_states(frames_by_state.keys().map(|k| k.as_str()), idle_frames),
    );
    let mut rows: BTreeMap<StateId, Vec<RgbaImage>> = BTreeMap::new();
    for sid in StateId::ALL {
        let Some(frames) = frames_by_state.get(sid.key()) else {
            continue;
        };
        if frames.is_empty() {
            continue;
        }
        let cells: Vec<RgbaImage> = frames
            .iter()
            .take(8)
            .map(|f| layout::fit_into_cell(f, scale))
            .collect();
        let fps = if sid == StateId::Idle && cells.len() == 1 {
            2
        } else {
            sid.default_fps()
        };
        spec.states.insert(
            sid.key().into(),
            StateClip {
                row: sid.row(),
                frames: cells.len() as u32,
                fps,
                looping: sid.default_loop(),
                mirror_of: None,
                // 导入的真素材：运行时必须原样播放（走路不得被程序化剪纸顶掉）
                source: Some(crate::petkit::SOURCE_REAL.into()),
            },
        );
        rows.insert(sid, cells);
    }

    if spec.states.contains_key("walk-right") && !spec.states.contains_key("walk-left") {
        let src = spec.states.get("walk-right").unwrap().clone();
        spec.states.insert(
            "walk-left".into(),
            StateClip {
                row: StateId::WalkLeft.row(),
                frames: src.frames,
                fps: src.fps,
                looping: src.looping,
                mirror_of: Some("walk-right".into()),
                source: src.source.clone(), // 镜像继承来源
            },
        );
    }

    Ok((spec, layout::compose_sheet(&rows)))
}

fn session_frames(session: &WizardSession) -> BTreeMap<String, Vec<RgbaImage>> {
    session
        .strips
        .iter()
        .map(|(k, s)| (k.clone(), s.frames.clone()))
        .collect()
}

pub fn preview(session: &WizardSession) -> PetResult<ComposeView> {
    let (mut spec, sheet) = compose_from_frames(&session_frames(session))?;
    spec.name = "预览".into();
    let report = validator::validate(&spec, &sheet);
    Ok(ComposeView {
        sheet: to_png_data_url(&sheet)?,
        report,
        tier: estimate_tier(session),
    })
}

// ---------------------------------------------------------------- 孵化（新宠物）

fn persist_sources(pets_root: &Path, pet_id: &str, session: &WizardSession) {
    for (key, strip) in &session.strips {
        let _ = store::save_source_frames(pets_root, pet_id, key, &strip.frames);
    }
}

/// 会话素材 → 新宠物。落盘成功后由胶水层负责「设为当前 + 广播」。
///
/// `seq` 是同秒防撞号计数(见 [`store::new_pet_id`])。
pub fn hatch(
    pets_root: &Path,
    seq: u32,
    session: &WizardSession,
    name: &str,
) -> PetResult<HatchResult> {
    let name = name.trim();
    if name.is_empty() {
        return Err(PetError::msg("请先给宠物起个名字"));
    }
    let (mut spec, sheet) = compose_from_frames(&session_frames(session))?;
    spec.name = name.to_string();
    let report = validator::validate(&spec, &sheet);
    if !report.ok {
        return Ok(HatchResult {
            ok: false,
            pet_id: None,
            report,
        });
    }
    let pet_id = store::save_pet(pets_root, seq, &spec, &sheet)?;
    persist_sources(pets_root, &pet_id, session);
    Ok(HatchResult {
        ok: true,
        pet_id: Some(pet_id),
        report,
    })
}

// ---------------------------------------------------------------- 进化（当前宠物）

/// 把会话素材增量合并进 `pet_id`（先孵化，后进化）。
///
/// 关键:合并的是**持久化的源帧**而非现有图集单元格 —— 全量参与统一缩放,
/// 任意次进化后各状态比例才一致。
pub fn evolve(pets_root: &Path, pet_id: &str, session: &WizardSession) -> PetResult<EvolveResult> {
    if session.strips.is_empty() {
        return Err(PetError::msg("还没有导入任何素材"));
    }

    let old_spec = store::load_spec(pets_root, pet_id)?;
    let mut merged = store::load_all_source_frames(pets_root, pet_id)?;
    for (key, strip) in &session.strips {
        merged.insert(key.clone(), strip.frames.clone());
    }

    let (mut spec, sheet) = compose_from_frames(&merged)?;
    spec.name = old_spec.name.clone();
    spec.author = old_spec.author.clone();
    spec.created = old_spec.created.clone();
    spec.anchors = old_spec.anchors.clone();

    let report = validator::validate(&spec, &sheet);
    if !report.ok {
        return Ok(EvolveResult {
            ok: false,
            pet_id: pet_id.to_string(),
            report,
            unlocked: Vec::new(),
        });
    }

    store::overwrite_pet(pets_root, pet_id, &spec, &sheet)?;
    persist_sources(pets_root, pet_id, session);

    let unlocked: Vec<String> = session
        .strips
        .keys()
        .filter_map(|k| StateId::from_key(k).map(|s| s.display_name().to_string()))
        .collect();
    Ok(EvolveResult {
        ok: true,
        pet_id: pet_id.to_string(),
        report,
        unlocked,
    })
}

// ---------------------------------------------------------------- L0 单图模式

/// L0 预览：抠背景后的单帧效果图（容差 None = 自动）。
pub fn l0_preview(path: &str, tolerance: Option<f32>) -> PetResult<serde_json::Value> {
    let data = process_image(path, tolerance, None, true)?;
    let frame = &data.frames[0];
    let scale = layout::global_scale(&[frame]);
    let cell = layout::fit_into_cell(frame, scale);
    Ok(serde_json::json!({
        "preview": to_png_data_url(&cell)?,
        "background": bg_label(data.background),
        "tolerance": data.tolerance,
        "warnings": data.warnings,
    }))
}

/// L0 孵化：单图 → idle 单帧宠物（先孵化的入口，≤60s 上桌面）。
pub fn l0_hatch(
    pets_root: &Path,
    seq: u32,
    path: &str,
    tolerance: Option<f32>,
    name: &str,
) -> PetResult<HatchResult> {
    let name = name.trim();
    if name.is_empty() {
        return Err(PetError::msg("请先给宠物起个名字"));
    }
    let data = process_image(path, tolerance, None, true)?;

    let mut frames_by_state = BTreeMap::new();
    frames_by_state.insert("idle".to_string(), vec![data.frames[0].clone()]);
    let (mut spec, sheet) = compose_from_frames(&frames_by_state)?;
    spec.name = name.to_string();
    spec.tier = Some("L0".into());

    let report = validator::validate(&spec, &sheet);
    if !report.ok {
        return Ok(HatchResult {
            ok: false,
            pet_id: None,
            report,
        });
    }
    let pet_id = store::save_pet(pets_root, seq, &spec, &sheet)?;
    let _ = store::save_source_frames(pets_root, &pet_id, "idle", &data.frames[..1]);
    Ok(HatchResult {
        ok: true,
        pet_id: Some(pet_id),
        report,
    })
}

// ---------------------------------------------------------------- 参考图 / 救帧

/// 文件名里不能出现的字符（Windows 最严，取其超集）。
const FILENAME_FORBIDDEN: [char; 9] = ['/', '\\', ':', '*', '?', '"', '<', '>', '|'];

/// 参考图的建议文件名（另存为对话框的默认值）。
pub fn reference_filename(pet_name: &str) -> String {
    let safe: String = pet_name
        .chars()
        .filter(|c| !FILENAME_FORBIDDEN.contains(c))
        .collect();
    let safe = safe.trim();
    if safe.is_empty() {
        "参考图.png".to_string()
    } else {
        format!("参考图_{safe}.png")
    }
}

/// 参考图导出：idle 首帧合成到白底 → `dest`。
///
/// 目标路径由调用方给（原实现硬编码 `~/HatchDesk/`，句流改走「另存为」，
/// 位置由用户自己定）。
pub fn export_reference(pets_root: &Path, pet_id: &str, dest: &Path) -> PetResult<()> {
    let sources = store::load_all_source_frames(pets_root, pet_id)?;
    let frame = sources
        .get("idle")
        .and_then(|f| f.first())
        .ok_or_else(|| PetError::msg("该宠物还没有 idle 素材"))?;

    // 白底合成：部分平台上传器对透明 PNG 显示为棋盘格，白底参考图对 AI 更友好
    let b = layout::bbox(frame).ok_or_else(|| PetError::msg("idle 首帧为空"))?;
    let pad = 24u32;
    let mut card = RgbaImage::from_pixel(
        b.w + pad * 2,
        b.h + pad * 2,
        image::Rgba([255, 255, 255, 255]),
    );
    let cropped = image::imageops::crop_imm(frame, b.x0, b.y0, b.w, b.h).to_image();
    image::imageops::overlay(&mut card, &cropped, pad as i64, pad as i64);

    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(dest, crate::imaging::encode_png(&card)?)?;
    Ok(())
}

/// 用已导入的原始文件按指定帧数重切（帧粘连时的救帧手段之一）。
pub fn reslice(session: &mut WizardSession, state_key: &str, forced_frames: u32) -> PetResult<()> {
    let (path, tolerance) = {
        let strip = session
            .strips
            .get(state_key)
            .ok_or_else(|| PetError::msg("该状态还没有导入素材"))?;
        (strip.source_path.clone(), strip.tolerance)
    };
    let data = process_image(&path, Some(tolerance), Some(forced_frames), false)?;
    session.strips.insert(state_key.to_string(), data);
    Ok(())
}

/// 单帧替换：用新图（取第一帧）替换指定状态的第 idx 帧。
pub fn replace_frame(
    session: &mut WizardSession,
    state_key: &str,
    frame_idx: usize,
    path: &str,
    tolerance: Option<f32>,
) -> PetResult<()> {
    let incoming = process_image(path, tolerance, None, false)?;
    let strip = session
        .strips
        .get_mut(state_key)
        .ok_or_else(|| PetError::msg("该状态还没有导入素材"))?;
    if frame_idx >= strip.frames.len() {
        return Err(PetError::msg("帧序号超出范围"));
    }
    strip.frames[frame_idx] = incoming.frames[0].clone();
    strip
        .warnings
        .push(format!("第 {} 帧已手动替换", frame_idx + 1));
    Ok(())
}

/// 导出某状态某帧 PNG（救帧参考图）。
pub fn export_frame(
    session: &WizardSession,
    state_key: &str,
    frame_idx: usize,
    dest: &str,
) -> PetResult<()> {
    let strip = session
        .strips
        .get(state_key)
        .ok_or_else(|| PetError::msg("该状态还没有导入素材"))?;
    let frame = strip
        .frames
        .get(frame_idx)
        .ok_or_else(|| PetError::msg("帧序号超出范围"))?;
    let png = crate::imaging::encode_png(frame)?;
    std::fs::write(dest, png)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn make_strip_file(frames: u32, dir: &std::path::Path, name: &str) -> String {
        let cell = 80u32;
        let mut img = RgbaImage::from_pixel(cell * frames, 100, Rgba([0, 255, 0, 255]));
        for f in 0..frames {
            let x0 = f * cell + 15;
            for y in 20..90 {
                for x in x0..x0 + 50 {
                    img.put_pixel(x, y, Rgba([200, 90, 60, 255]));
                }
            }
        }
        let path = dir.join(name);
        img.save(&path).unwrap();
        path.to_string_lossy().to_string()
    }

    /// 造 2 行 × N 列绿幕网格图（idle+walk 双行版式）。
    fn make_grid_file(rows: u32, cols: u32, dir: &std::path::Path, name: &str) -> String {
        let (cw, ch) = (80u32, 100u32);
        let mut img = RgbaImage::from_pixel(cw * cols + 60, ch * rows + 60, Rgba([0, 255, 0, 255]));
        for r in 0..rows {
            for c in 0..cols {
                let x0 = 30 + c * cw + 15;
                let y0 = 30 + r * ch + 20;
                for y in y0..y0 + 60 {
                    for x in x0..x0 + 50 {
                        img.put_pixel(x, y, Rgba([200, (90 + r * 40) as u8, 60, 255]));
                    }
                }
            }
        }
        let path = dir.join(name);
        img.save(&path).unwrap();
        path.to_string_lossy().to_string()
    }

    #[test]
    fn full_pipeline_green_strip_to_sheet() {
        let dir = std::env::temp_dir().join("sf-pet-test-wizard");
        std::fs::create_dir_all(&dir).unwrap();
        let idle = make_strip_file(6, &dir, "idle.png");
        let walk = make_strip_file(8, &dir, "walk.png");

        let mut session = WizardSession::default();
        session.strips.insert(
            "idle".into(),
            process_image(&idle, None, None, false).unwrap(),
        );
        session.strips.insert(
            "walk-right".into(),
            process_image(&walk, None, None, false).unwrap(),
        );

        assert_eq!(session.strips["idle"].frames.len(), 6);
        assert_eq!(session.strips["walk-right"].frames.len(), 8);

        let (mut spec, sheet) = compose_from_frames(&session_frames(&session)).unwrap();
        spec.name = "测试".into();
        assert_eq!(
            spec.states["walk-left"].mirror_of.as_deref(),
            Some("walk-right")
        );
        let report = validator::validate(&spec, &sheet);
        assert!(report.ok, "findings: {:?}", report.findings);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn grid_sheet_import_lights_two_states() {
        let dir = std::env::temp_dir().join("sf-pet-test-sheet");
        std::fs::create_dir_all(&dir).unwrap();
        let grid = make_grid_file(2, 6, &dir, "grid.png");

        let mut session = WizardSession::default();
        let imported = add_sheet(
            &mut session,
            &grid,
            &["idle".to_string(), "walk-right".to_string()],
            Some(6),
            &chroma::default_corner_rois(),
        )
        .unwrap();
        assert_eq!(imported, vec!["idle", "walk-right"]);
        assert_eq!(session.strips["idle"].frames.len(), 6);
        assert_eq!(session.strips["walk-right"].frames.len(), 6);

        // 整图导入的产物可直接合成通过校验
        let (mut spec, sheet) = compose_from_frames(&session_frames(&session)).unwrap();
        spec.name = "网格".into();
        assert!(validator::validate(&spec, &sheet).ok);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn swap_states_exchanges_rows() {
        let dir = std::env::temp_dir().join("sf-pet-test-swap");
        std::fs::create_dir_all(&dir).unwrap();
        let grid = make_grid_file(2, 4, &dir, "grid.png");
        let mut session = WizardSession::default();
        add_sheet(
            &mut session,
            &grid,
            &["idle".to_string(), "walk-right".to_string()],
            Some(4),
            &[],
        )
        .unwrap();
        let idle_hash = phash::phash(&session.strips["idle"].frames[0]);
        swap_states(&mut session, "idle", "walk-right").unwrap();
        assert_eq!(
            phash::phash(&session.strips["walk-right"].frames[0]),
            idle_hash
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 导入的素材必须标成真素材，镜像态继承。
    ///
    /// 运行时靠这个标记决定走路怎么画：标记丢了，用户从视频里抠出来的行走/奔跑
    /// 就会被程序化的分层剪纸走路顶掉（表现为「宠物不按导入的动作活动」）。
    #[test]
    fn composed_states_are_marked_real() {
        let mut frames = BTreeMap::new();
        let cell = |v: u8| {
            let mut img = RgbaImage::new(80, 100);
            for y in 20..90 {
                for x in 15..65 {
                    img.put_pixel(x, y, Rgba([200, v, 60, 255]));
                }
            }
            img
        };
        frames.insert("idle".to_string(), vec![cell(90), cell(95)]);
        frames.insert("walk-right".to_string(), vec![cell(120), cell(130)]);

        let (spec, _) = compose_from_frames(&frames).unwrap();
        for key in ["idle", "walk-right"] {
            assert_eq!(
                spec.states[key].source.as_deref(),
                Some(crate::petkit::SOURCE_REAL),
                "{key} 应标记为真素材"
            );
        }
        assert_eq!(
            spec.states["walk-left"].source.as_deref(),
            Some(crate::petkit::SOURCE_REAL),
            "镜像出的左走应继承右走的来源"
        );
    }

    #[test]
    fn compose_without_idle_fails() {
        let session = WizardSession::default();
        assert!(compose_from_frames(&session_frames(&session)).is_err());
    }

    /// 端到端：真实视频 → 视频进化产物（需本机 ffmpeg）。
    /// 用法：`SF_PET_TEST_VIDEO=<mp4 路径> cargo test --release video_evolution_smoke -- --ignored --nocapture`
    /// 会把每帧与拼好的动画条写到 `SF_PET_TEST_OUT`（缺省为临时目录），便于肉眼验收。
    #[test]
    #[ignore]
    fn video_evolution_smoke() {
        let Ok(video) = std::env::var("SF_PET_TEST_VIDEO") else {
            eprintln!("跳过：未设置 SF_PET_TEST_VIDEO");
            return;
        };
        let out_dir = std::env::var("SF_PET_TEST_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("sf-pet-video-smoke"));
        std::fs::create_dir_all(&out_dir).unwrap();

        let t0 = std::time::Instant::now();
        let progress = |stage: &str, done: usize, total: usize| {
            if total <= 1 || done == total || done % 4 == 0 {
                eprintln!("  [{stage}] {done}/{total}");
            }
        };
        let data = process_video(None, &video, 6, &chroma::default_corner_rois(), &progress)
            .expect("真实视频应能抽出动画帧");
        eprintln!(
            "耗时 {:?}；背景判定 = {}；抽出 {} 帧",
            t0.elapsed(),
            bg_label(data.background),
            data.frames.len()
        );
        for w in &data.warnings {
            eprintln!("  ⚠ {w}");
        }

        // 前景占比：碎裂的抠图会只剩零星碎片，这里必须是一个完整主体
        let refs: Vec<&RgbaImage> = data.frames.iter().collect();
        let scale = layout::global_scale(&refs);
        let cells: Vec<RgbaImage> = data
            .frames
            .iter()
            .map(|f| layout::fit_into_cell(f, scale))
            .collect();

        let mut strip = RgbaImage::new(
            crate::petkit::CELL_W * cells.len() as u32,
            crate::petkit::CELL_H,
        );
        for (i, c) in cells.iter().enumerate() {
            image::imageops::overlay(&mut strip, c, (i as u32 * crate::petkit::CELL_W) as i64, 0);
        }
        let strip_path = out_dir.join("video_strip.png");
        strip.save(&strip_path).unwrap();
        eprintln!("动画条已写出：{}", strip_path.display());

        // 主体连通性检查：抠图碎裂时最大连通域只占前景的一小部分
        for (i, f) in data.frames.iter().enumerate() {
            let frac = matting::foreground_fraction(f);
            let share = largest_component_share(f);
            eprintln!(
                "  帧{i}: 前景占比 {:.1}%，最大连通域占前景 {:.1}%",
                frac * 100.0,
                share * 100.0
            );
            assert!(
                share > 0.5,
                "第 {i} 帧抠图碎裂：最大连通域只占前景 {:.1}%（形象损坏的特征）",
                share * 100.0
            );
        }
        assert!(data.frames.len() >= 2);
    }

    /// 端到端：一次生成、多状态点亮（需本机 ffmpeg + `SF_PET_TEST_VIDEO`）。
    #[test]
    #[ignore]
    fn video_segments_smoke() {
        let Ok(video) = std::env::var("SF_PET_TEST_VIDEO") else {
            eprintln!("跳过：未设置 SF_PET_TEST_VIDEO");
            return;
        };
        let out_dir = std::env::var("SF_PET_TEST_OUT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("sf-pet-video-smoke"));
        std::fs::create_dir_all(&out_dir).unwrap();

        // 默认双段；SF_PET_TEST_PLAN=full 时跑「完全体」四段脚本
        let segments = if std::env::var("SF_PET_TEST_PLAN").as_deref() == Ok("full") {
            vec![
                VideoSegment {
                    state_key: "idle".into(),
                    start: 0.0,
                    end: 0.25,
                    target_frames: 6,
                },
                VideoSegment {
                    state_key: "walk-right".into(),
                    start: 0.25,
                    end: 0.5,
                    target_frames: 6,
                },
                VideoSegment {
                    state_key: "happy".into(),
                    start: 0.5,
                    end: 0.75,
                    target_frames: 5,
                },
                VideoSegment {
                    state_key: "sleep".into(),
                    start: 0.75,
                    end: 1.0,
                    target_frames: 4,
                },
            ]
        } else {
            vec![
                VideoSegment {
                    state_key: "idle".into(),
                    start: 0.0,
                    end: 0.5,
                    target_frames: 6,
                },
                VideoSegment {
                    state_key: "walk-right".into(),
                    start: 0.5,
                    end: 1.0,
                    target_frames: 6,
                },
            ]
        };
        let t0 = std::time::Instant::now();
        let progress = |stage: &str, done: usize, total: usize| {
            if total <= 1 || done == total {
                eprintln!("  [{stage}] {done}/{total}");
            }
        };
        let out = process_video_segments(
            None,
            &video,
            &segments,
            &chroma::default_corner_rois(),
            &progress,
        )
        .expect("分段导入应成功");
        eprintln!("耗时 {:?}；切出 {} 段", t0.elapsed(), out.len());
        assert_eq!(out.len(), segments.len(), "每段脚本都应切出一条动画");

        // 全部段共用一套缩放，否则同一只宠物在各状态间会忽大忽小
        let all: Vec<&RgbaImage> = out.iter().flat_map(|(_, s)| s.frames.iter()).collect();
        let scale = layout::global_scale(&all);

        eprintln!(
            "\n{:<12} {:>4}  {:>9}  {:>9}  {:>9}  {:>8}",
            "状态", "帧", "姿态变化", "峰值", "最大跳变", "判定"
        );
        for (key, strip) in &out {
            assert!(strip.frames.len() >= 2, "{key} 至少要 2 帧");
            for (i, f) in strip.frames.iter().enumerate() {
                let share = largest_component_share(f);
                assert!(
                    share > 0.5,
                    "{key} 第 {i} 帧抠图碎裂（最大连通域仅占 {share:.2}）"
                );
            }

            // —— 生动性判据 ——
            // 关键：在 `fit_into_cell` **之后**度量。入格会把主体按质心水平居中、
            // 底边对齐，也就是**把整体平移彻底抹掉**。所以此处剩下的差异只可能来自
            // 姿态本身（迈腿、起伏、转头、趴下）。若素材只是"整只狗横向滑过画面"，
            // 入格后各帧会几乎相同，这个数字就会趴到 0 —— 正是要防的"简单平移"。
            let cells: Vec<RgbaImage> = strip
                .frames
                .iter()
                .map(|f| layout::fit_into_cell(f, scale))
                .collect();
            let steps: Vec<f32> = (0..cells.len() - 1)
                .map(|i| motion::motion_between(&cells[i], &cells[i + 1]))
                .collect();
            let mean = steps.iter().sum::<f32>() / steps.len() as f32;
            let peak = (1..cells.len())
                .map(|i| motion::motion_between(&cells[0], &cells[i]))
                .fold(0.0, f32::max);

            // —— 连贯性判据 ——
            // 只有下限是不够的：一段横跨"侧走→正面→趴下"的素材，逐帧差异会**特别大**，
            // 分数很高却根本不是一个动作，播出来是一帧一个样。故同时卡上限。
            let max_step = steps.iter().fold(0.0f32, |a, b| a.max(*b));
            let verdict = if mean < LIVELY_MIN {
                "偏平"
            } else if max_step > COHERENT_MAX {
                "跳变"
            } else {
                "生动"
            };
            eprintln!(
                "{key:<12} {:>4}  {mean:>9.4}  {peak:>9.4}  {max_step:>9.4}  {verdict:>8}",
                strip.frames.len()
            );
            assert!(
                mean >= LIVELY_MIN,
                "{key} 入格后逐帧姿态变化仅 {mean:.4}（下限 {LIVELY_MIN}）——\
                 说明这段素材基本只是整体平移，动画会显得很死板"
            );
            assert!(
                max_step <= COHERENT_MAX,
                "{key} 相邻帧最大跳变 {max_step:.4}（上限 {COHERENT_MAX}）——\
                 这段帧不属于同一个动作（多半横跨了动作切换），播出来会一帧一个样"
            );

            let mut png = RgbaImage::new(
                crate::petkit::CELL_W * cells.len() as u32,
                crate::petkit::CELL_H,
            );
            for (i, c) in cells.iter().enumerate() {
                image::imageops::overlay(&mut png, c, (i as u32 * crate::petkit::CELL_W) as i64, 0);
            }
            png.save(out_dir.join(format!("seg_{key}.png"))).unwrap();
        }
        eprintln!();
        for (key, strip) in &out {
            for w in &strip.warnings {
                eprintln!("  [{key}] ⚠ {w}");
            }
        }
        eprintln!("\n分段动画条已写出到 {}", out_dir.display());
    }

    /// 入格后逐帧姿态变化的下限。低于此值说明素材几乎只有整体位移、没有姿态变化，
    /// 动画会像"贴图在滑动"。标定参考：纯静止片段约 0.000x，真实步态约 0.02–0.05。
    const LIVELY_MIN: f32 = 0.012;
    /// 相邻帧跳变的上限。真实步态的逐帧跨度不会超过这个量级；
    /// 超了说明这几帧根本不是同一个动作（横跨了动作切换）。
    const COHERENT_MAX: f32 = 0.35;

    /// 最大不透明连通域占全部不透明像素的比例。
    /// 1.0 = 一个完整主体；接近 0 = 碎成了一地（旧泛洪抠图的产物）。
    #[cfg(test)]
    fn largest_component_share(img: &RgbaImage) -> f32 {
        use crate::imaging::ALPHA_SOLID;
        let (w, h) = img.dimensions();
        let idx = |x: u32, y: u32| (y * w + x) as usize;
        let mut seen = vec![false; (w * h) as usize];
        let (mut total, mut best) = (0u32, 0u32);
        for sy in 0..h {
            for sx in 0..w {
                if seen[idx(sx, sy)] || img.get_pixel(sx, sy).0[3] < ALPHA_SOLID {
                    continue;
                }
                let mut q = std::collections::VecDeque::from([(sx, sy)]);
                seen[idx(sx, sy)] = true;
                let mut size = 0u32;
                while let Some((x, y)) = q.pop_front() {
                    size += 1;
                    for (nx, ny) in [
                        (x.wrapping_sub(1), y),
                        (x + 1, y),
                        (x, y.wrapping_sub(1)),
                        (x, y + 1),
                    ] {
                        if nx < w
                            && ny < h
                            && !seen[idx(nx, ny)]
                            && img.get_pixel(nx, ny).0[3] >= ALPHA_SOLID
                        {
                            seen[idx(nx, ny)] = true;
                            q.push_back((nx, ny));
                        }
                    }
                }
                total += size;
                best = best.max(size);
            }
        }
        if total == 0 {
            0.0
        } else {
            best as f32 / total as f32
        }
    }

    /// 出画检测：角色贴边（有一部分在取景框外）必须被识别出来。
    /// 这类素材做成精灵会缺一块，且无法从像素上补回，只能提示用户重出该段。
    #[test]
    fn detects_subject_cropped_by_frame_edge() {
        // 完整在画面内
        let mut inside = RgbaImage::new(200, 200);
        for y in 40..160 {
            for x in 40..160 {
                inside.put_pixel(x, y, Rgba([200, 150, 100, 255]));
            }
        }
        assert!(!touches_frame_edge(&inside), "完整主体不应判为出画");

        // 左侧被画面切掉（正是走路素材里"狗的后半身在画面外"的形态）
        let mut cropped = RgbaImage::new(200, 200);
        for y in 40..160 {
            for x in 0..120 {
                cropped.put_pixel(x, y, Rgba([200, 150, 100, 255]));
            }
        }
        assert!(touches_frame_edge(&cropped), "贴左边缘应判为出画");

        // 底部贴边 = 站在地面上，是正常取景，绝不能误报
        // （实测待机段 6/6 帧都触底，若把底边算进去就会满屏假警告）
        let mut standing = RgbaImage::new(200, 200);
        for y in 80..200 {
            for x in 60..140 {
                standing.put_pixel(x, y, Rgba([200, 150, 100, 255]));
            }
        }
        assert!(
            !touches_frame_edge(&standing),
            "站在画面底边上属正常取景，不应误报"
        );

        // 顶部贴边（头顶被切）要算
        let mut top = RgbaImage::new(200, 200);
        for y in 0..120 {
            for x in 60..140 {
                top.put_pixel(x, y, Rgba([200, 150, 100, 255]));
            }
        }
        assert!(touches_frame_edge(&top), "头顶被切应判为出画");
    }

    /// 回归：浅色主体 + 浅色纯色背景不能被泛洪吃穿。
    ///
    /// 泛洪的颜色距离阈值是 `28 + 1.6 × 容差`，纯色背景的自动容差 45（视频源 60）
    /// 对应阈值 100（124）—— 而奶油色主体与浅灰背景的距离只有约 72，
    /// 于是泛洪从边界一路穿进主体，只剩最深的几块碎片。这正是形象损坏的同一类根因。
    #[test]
    fn light_subject_on_light_solid_background_survives() {
        let mut img = RgbaImage::from_pixel(240, 240, Rgba([232, 232, 232, 255]));
        for y in 60..210 {
            for x in 80..160 {
                img.put_pixel(x, y, Rgba([235, 205, 165, 255])); // 奶油色主体
            }
        }
        for y in 90..110 {
            for x in 100..120 {
                img.put_pixel(x, y, Rgba([60, 45, 35, 255])); // 深色五官
            }
        }
        assert_eq!(
            chroma::detect_background(&img),
            Background::Solid,
            "前提：应判为纯色背景"
        );

        let subject_share = (150.0 * 80.0) / (240.0 * 240.0); // ≈ 0.208
        let cut = remove_background(img, None, SourceKind::Image, false);
        let kept = matting::foreground_fraction(&cut.img);
        assert!(
            kept > subject_share * 0.6,
            "主体被吃穿：应保住约 {:.1}% 的画面，实际只剩 {:.2}%",
            subject_share * 100.0,
            kept * 100.0
        );
    }

    #[test]
    fn l0_single_image_auto_tolerance() {
        let dir = std::env::temp_dir().join("sf-pet-test-l0");
        std::fs::create_dir_all(&dir).unwrap();
        let mut img = RgbaImage::from_pixel(300, 300, Rgba([245, 245, 245, 255]));
        for y in 60..240 {
            for x in 80..220 {
                img.put_pixel(x, y, Rgba([120, 90, 200, 255]));
            }
        }
        let path = dir.join("photo.png");
        img.save(&path).unwrap();
        let data = process_image(path.to_str().unwrap(), None, None, true).unwrap();
        assert_eq!(data.frames.len(), 1);
        assert_eq!(data.background, Background::Solid);
        assert!(data.tolerance > 0.0, "自动容差应生效");
        std::fs::remove_dir_all(&dir).ok();
    }
}

#[cfg(test)]
mod export_tests {
    use super::*;

    /// 另存为的默认文件名必须能落到 Windows 文件系统上
    /// （宠物名是用户随手起的，`:`、`?` 之类完全可能出现）。
    #[test]
    fn reference_filename_is_filesystem_safe() {
        assert_eq!(reference_filename("小咪"), "参考图_小咪.png");
        assert_eq!(reference_filename("a/b:c*d?"), "参考图_abcd.png");
        assert_eq!(
            reference_filename("  "),
            "参考图.png",
            "全被过滤后要有兜底名"
        );
        assert_eq!(reference_filename("///"), "参考图.png");
    }

    /// 参考图导出:白底、按 idle 首帧包围盒裁剪 + 24px 留白。
    #[test]
    fn export_reference_writes_a_white_card() {
        let root = std::env::temp_dir().join("sf-pet-wizard-reference");
        let _ = std::fs::remove_dir_all(&root);
        store::ensure_root(&root).unwrap();

        // 造一只只有 idle 的宠物,并留下源帧
        let mut frames = BTreeMap::new();
        let mut cell = RgbaImage::new(80, 100);
        for y in 20..90 {
            for x in 15..65 {
                cell.put_pixel(x, y, image::Rgba([200, 90, 60, 255]));
            }
        }
        frames.insert("idle".to_string(), vec![cell.clone()]);
        let (mut spec, sheet) = compose_from_frames(&frames).unwrap();
        spec.name = "小咪".into();
        let id = store::save_pet(&root, 0, &spec, &sheet).unwrap();
        store::save_source_frames(&root, &id, "idle", &[cell]).unwrap();

        let dest = root.join(reference_filename(&spec.name));
        export_reference(&root, &id, &dest).unwrap();
        let card = image::open(&dest).unwrap().to_rgba8();
        assert_eq!(
            card.dimensions(),
            (50 + 48, 70 + 48),
            "应是包围盒 + 两侧 24px"
        );
        assert_eq!(card.get_pixel(0, 0).0, [255, 255, 255, 255], "留白应为白底");
        std::fs::remove_dir_all(&root).ok();
    }
}
