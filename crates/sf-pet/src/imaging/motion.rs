//! 运动量度量与循环窗口检测（视频路线的选帧依据）。
//!
//! **为什么不能拿 pHash 当"动作幅度"判据**：pHash 是*身份不变量*
//! （32×32 灰度 → DCT 只取低频 8×8），它的设计目标恰恰是对呼吸、眨眼这类
//! 细微形变免疫。实测一段 10 秒待机视频，相邻帧汉明距离恒为 0–2，
//! 远低于原去重阈值 5 —— 于是整段被去重成 1 帧，误报「动作幅度过小」。
//! 这里改用主体范围内的归一化像素差，直接度量"动了多少"；
//! pHash 仍留在 `phash` 模块做它本职的身份漂移检测。
//!
//! 阈值标定（同一段豆包素材实测）：
//!   真实待机动作 peak = 0.01513，纯静止图 H.264 压缩噪声 peak = 0.00009，
//!   相差 168 倍，故 `STATIC_FLOOR` 取 0.0015（噪声的 16 倍、真信号的 1/10）。

use image::{RgbaImage, imageops};

use super::{ALPHA_SOLID, layout};

/// 分析用缩略图尺寸（与单元格 192×208 同比例）。循环搜索是 O(n²) 次比较，
/// 必须在小图上做：100 帧约 3400 次比较 × 4416 像素，毫秒级。
pub const THUMB_W: u32 = 64;
pub const THUMB_H: u32 = 69;

/// 低于此运动量视为「整段就是一张静止图」（见模块头的标定说明）。
pub const STATIC_FLOOR: f32 = 0.0015;

/// 首尾错配不超过窗口内平均步幅的这个倍数 → 判为真闭环。
pub const LOOP_TOLERANCE: f32 = 1.6;

/// 评分在最优值这个倍数以内的候选中取周期最长者：
/// 一次完整呼吸胜过一次局部抖动（否则搜索总会退化到最短周期）。
const PERIOD_PREFERENCE: f32 = 1.3;

/// 一个循环窗口至少要这么多帧（10fps 下 = 0.6 秒），再短就是抖动不是动作。
pub const MIN_LOOP_FRAMES: usize = 6;

/// 帧间运动量（0..1）：在两帧主体的**并集**内比较预乘亮度与 alpha，
/// 按并集面积归一化，因此与分辨率、主体大小无关。
/// 对不透明的原始视频帧（alpha 恒 255）同样适用，此时即为 ROI 内的平均亮度差。
pub fn motion_between(a: &RgbaImage, b: &RgbaImage) -> f32 {
    if a.dimensions() != b.dimensions() {
        return 1.0;
    }
    let mut acc = 0f64;
    let mut count = 0u64;
    for (pa, pb) in a.pixels().zip(b.pixels()) {
        let [ra, ga, ba, aa] = pa.0;
        let [rb, gb, bb, ab] = pb.0;
        if aa < ALPHA_SOLID && ab < ALPHA_SOLID {
            continue;
        }
        let (fa, fb) = (aa as f64 / 255.0, ab as f64 / 255.0);
        let la = (0.299 * ra as f64 + 0.587 * ga as f64 + 0.114 * ba as f64) * fa;
        let lb = (0.299 * rb as f64 + 0.587 * gb as f64 + 0.114 * bb as f64) * fb;
        acc += (la - lb).abs() + (fa - fb).abs() * 255.0;
        count += 1;
    }
    if count == 0 {
        return 0.0;
    }
    (acc / (count as f64 * 255.0 * 2.0)) as f32
}

/// 已抠像帧 → 分析缩略图：先按**全片统一缩放**入格（与 `layout::fit_into_cell`
/// 同一套规则），再缩到缩略尺寸。这样"姿态变了"才算动，"整体走过去了"不算 ——
/// 后者本来就会被入格时的质心居中抹掉，不该影响选帧。
pub fn thumbs_from_cut(frames: &[RgbaImage]) -> Vec<RgbaImage> {
    let refs: Vec<&RgbaImage> = frames.iter().collect();
    let scale = layout::global_scale(&refs);
    frames
        .iter()
        .map(|f| {
            imageops::resize(
                &layout::fit_into_cell(f, scale),
                THUMB_W,
                THUMB_H,
                imageops::FilterType::Triangle,
            )
        })
        .collect()
}

/// 原始（未抠像）帧 → 分析缩略图：裁到主体 ROI 再缩放。
/// 视频路线用它把运动分析和 AI 抠像解耦——分析在原始帧上零成本完成，
/// 只有最终选中的那几帧才付推理代价。
pub fn thumbs_from_roi(frames: &[RgbaImage], roi: &layout::Bbox) -> Vec<RgbaImage> {
    frames
        .iter()
        .map(|f| {
            let (w, h) = f.dimensions();
            let x = roi.x0.min(w.saturating_sub(1));
            let y = roi.y0.min(h.saturating_sub(1));
            let cw = roi.w.min(w - x).max(1);
            let ch = roi.h.min(h - y).max(1);
            let crop = imageops::crop_imm(f, x, y, cw, ch).to_image();
            imageops::resize(&crop, THUMB_W, THUMB_H, imageops::FilterType::Triangle)
        })
        .collect()
}

/// 检测到的循环窗口 `[start, start+period)`。
#[derive(Debug, Clone, Copy)]
pub struct LoopWindow {
    pub start: usize,
    pub period: usize,
    /// 首尾帧错配量
    pub mismatch: f32,
    /// 窗口内平均逐帧步幅
    pub inner_step: f32,
}

impl LoopWindow {
    /// 首尾错配相对于窗口内正常步幅的倍数；≤ `LOOP_TOLERANCE` 视为真闭环。
    pub fn score(&self) -> f32 {
        if self.inner_step <= f32::EPSILON {
            f32::MAX
        } else {
            self.mismatch / self.inner_step
        }
    }

    pub fn closed(&self) -> bool {
        self.score() <= LOOP_TOLERANCE
    }
}

/// 在缩略图序列里搜索最佳循环窗口。
/// 对每个候选周期取"首尾最像"的起点，用「首尾错配 ÷ 窗口内平均步幅」评分——
/// 除以内部步幅是为了让评分对动作快慢免疫（走路步幅大，呼吸步幅小，同等对待）。
pub fn best_loop(thumbs: &[RgbaImage], min_period: usize) -> Option<LoopWindow> {
    let n = thumbs.len();
    if n < 4 {
        return None;
    }
    let max_period = n / 2;
    let min_period = min_period.clamp(2, max_period);
    if min_period > max_period {
        return None;
    }

    let steps: Vec<f32> = (0..n - 1)
        .map(|i| motion_between(&thumbs[i], &thumbs[i + 1]))
        .collect();

    let mut candidates: Vec<LoopWindow> = Vec::new();
    for period in min_period..=max_period {
        let mut arg = 0usize;
        let mut lo = f32::MAX;
        for s in 0..n - period {
            let m = motion_between(&thumbs[s], &thumbs[s + period]);
            if m < lo {
                lo = m;
                arg = s;
            }
        }
        let inner = steps[arg..arg + period].iter().sum::<f32>() / period as f32;
        if inner <= f32::EPSILON {
            continue;
        }
        candidates.push(LoopWindow {
            start: arg,
            period,
            mismatch: lo,
            inner_step: inner,
        });
    }

    let best_score = candidates
        .iter()
        .map(|w| w.score())
        .fold(f32::MAX, f32::min);
    if !best_score.is_finite() {
        return None;
    }
    let cutoff = (best_score * PERIOD_PREFERENCE).max(best_score + 0.05);
    candidates
        .into_iter()
        .filter(|w| w.score() <= cutoff)
        .max_by_key(|w| w.period)
}

/// 相对某一帧的最大运动量（判断整段是否根本没动）。
pub fn peak_motion(thumbs: &[RgbaImage]) -> f32 {
    if thumbs.len() < 2 {
        return 0.0;
    }
    thumbs[1..]
        .iter()
        .map(|t| motion_between(&thumbs[0], t))
        .fold(0.0, f32::max)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    /// 与 decoder 单测同款素材：固定画布上一块方形主体。
    fn blob(cx: u32, size: u32) -> RgbaImage {
        let mut img = RgbaImage::new(200, 120);
        for y in 30..(30 + size).min(120) {
            for x in cx..(cx + size).min(200) {
                img.put_pixel(x, y, Rgba([200, 90, 60, 255]));
            }
        }
        img
    }

    #[test]
    fn identical_frames_have_zero_motion() {
        let a = blob(80, 44);
        assert_eq!(motion_between(&a, &a.clone()), 0.0);
    }

    #[test]
    fn motion_grows_with_pose_change() {
        let base = blob(80, 40);
        let small = motion_between(&base, &blob(80, 42));
        let large = motion_between(&base, &blob(80, 60));
        assert!(small > 0.0 && large > small, "small={small} large={large}");
    }

    #[test]
    fn pure_translation_is_not_motion_after_fitting() {
        // 只是整体平移：入格时质心居中会抹掉它，不该被算作"动作"
        let frames = vec![blob(30, 44), blob(90, 44)];
        let t = thumbs_from_cut(&frames);
        assert!(motion_between(&t[0], &t[1]) < 1e-4, "平移不应计入运动量");
    }

    #[test]
    fn breathing_loop_is_detected_and_closed() {
        let sizes = [40u32, 46, 52, 46, 40, 46, 52, 46, 40];
        let frames: Vec<RgbaImage> = sizes.iter().map(|&s| blob(80, s)).collect();
        let t = thumbs_from_cut(&frames);
        let w = best_loop(&t, MIN_LOOP_FRAMES).expect("应找到循环窗口");
        assert!(w.closed(), "呼吸循环应判为闭环，score={}", w.score());
        assert_eq!(w.period, 4, "周期应为 4 帧");
    }

    #[test]
    fn monotonic_sequence_is_not_a_loop() {
        let frames: Vec<RgbaImage> = (0..8).map(|i| blob(30 + i * 12, 40 + i * 5)).collect();
        let t = thumbs_from_cut(&frames);
        let w = best_loop(&t, MIN_LOOP_FRAMES).expect("总会返回一个候选");
        assert!(!w.closed(), "单调变化不应判为闭环，score={}", w.score());
    }

    #[test]
    fn static_clip_has_peak_below_floor() {
        let frames: Vec<RgbaImage> = (0..10).map(|_| blob(80, 44)).collect();
        let t = thumbs_from_cut(&frames);
        assert!(peak_motion(&t) < STATIC_FLOOR);
    }

    #[test]
    fn moving_clip_has_peak_above_floor() {
        let sizes = [40u32, 46, 52, 46, 40];
        let frames: Vec<RgbaImage> = sizes.iter().map(|&s| blob(80, s)).collect();
        let t = thumbs_from_cut(&frames);
        assert!(peak_motion(&t) > STATIC_FLOOR);
    }
}
