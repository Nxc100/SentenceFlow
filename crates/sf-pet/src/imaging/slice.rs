//! 切帧：投影法内容感知切分（一维横条 + 二维网格），失败回退等宽/等高切分。
//!
//! 二维切分（改进方案 §6）：先对「行」做水平投影得行带，再对每个行带做列投影，
//! 行数/列数与版式声明不符时逐级回退（行等高 / 列等宽），沿用一维时代的告警语义。

use image::RgbaImage;

use super::ALPHA_SOLID;

/// 一段连续区间 [a, b)（列切分时为 x 区间，行切分时为 y 区间）。
pub type Span = (u32, u32);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Axis {
    /// 沿 X 轴切列（统计每一列的不透明像素）
    X,
    /// 沿 Y 轴切行（统计每一行的不透明像素）
    Y,
}

/// 投影法：统计垂直于 `axis` 方向的不透明像素数，空白串作为分隔。
pub fn detect_spans_axis(img: &RgbaImage, axis: Axis) -> Vec<Span> {
    let (w, h) = img.dimensions();
    let (main_len, cross_len) = match axis {
        Axis::X => (w, h),
        Axis::Y => (h, w),
    };
    let mut occupancy = vec![0u32; main_len as usize];
    for y in 0..h {
        for x in 0..w {
            if img.get_pixel(x, y).0[3] >= ALPHA_SOLID {
                let idx = match axis {
                    Axis::X => x,
                    Axis::Y => y,
                };
                occupancy[idx as usize] += 1;
            }
        }
    }
    // 低于横截面 0.5% 的位置视为空白（容忍零星噪点）
    let noise_floor = ((cross_len as f32) * 0.005).ceil() as u32;

    let mut spans: Vec<Span> = Vec::new();
    let mut start: Option<u32> = None;
    for i in 0..main_len {
        let occupied = occupancy[i as usize] > noise_floor;
        match (occupied, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                spans.push((s, i));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        spans.push((s, main_len));
    }

    // 合并间隙 <=2px 的相邻段（角色内部的细缝不算分隔）
    let mut merged: Vec<Span> = Vec::new();
    for span in spans {
        match merged.last_mut() {
            Some(last) if span.0.saturating_sub(last.1) <= 2 => last.1 = span.1,
            _ => merged.push(span),
        }
    }
    // 过滤过窄碎片（<1% 或 <4px）
    let min_len = ((main_len as f32) * 0.01).max(4.0) as u32;
    merged.retain(|(a, b)| b - a >= min_len);
    merged
}

/// 兼容旧 API：列投影。
pub fn detect_spans(img: &RgbaImage) -> Vec<Span> {
    detect_spans_axis(img, Axis::X)
}

/// 等宽（或等高）切分。
pub fn uniform_spans(length: u32, n: u32) -> Vec<Span> {
    let n = n.max(1);
    (0..n)
        .map(|i| (length * i / n, length * (i + 1) / n))
        .collect()
}

/// 一维综合策略：自动检测；给定 forced 且与检测不符时回退等宽。
/// 返回 (spans, used_uniform_fallback)。
pub fn slice_strip(img: &RgbaImage, forced: Option<u32>) -> (Vec<Span>, bool) {
    let auto = detect_spans(img);
    match forced {
        Some(n) if n >= 1 && auto.len() as u32 != n => (uniform_spans(img.width(), n), true),
        Some(_) | None if !auto.is_empty() => (auto, false),
        _ => (uniform_spans(img.width(), 1), true),
    }
}

/// 从横条中按区间裁出各帧（保持整条高度）。
pub fn cut_frames(img: &RgbaImage, spans: &[Span]) -> Vec<RgbaImage> {
    spans
        .iter()
        .map(|&(x0, x1)| image::imageops::crop_imm(img, x0, 0, x1 - x0, img.height()).to_image())
        .collect()
}

/// 二维切分的回退记录（进入向导告警文案）。
#[derive(Clone, Copy, Default, Debug)]
pub struct GridFallback {
    /// 行检测与声明不符，按等高回退
    pub uniform_rows: bool,
    /// 至少一行的帧检测与声明不符，按等宽回退
    pub uniform_cols: bool,
}

/// 二维网格切分（改进方案 §6.1）：
/// 1. 行带检测（Y 轴投影）；行数与 `expected_rows` 不符 → 等高回退；
/// 2. 每行带内列检测；`expected_cols` 给定且不符 → 该行等宽回退；
/// 3. 返回 rows × frames 的帧矩阵（帧保留行带全高，后续 fit_into_cell 按 bbox 对齐）。
pub fn slice_grid(
    img: &RgbaImage,
    expected_rows: u32,
    expected_cols: Option<u32>,
) -> (Vec<Vec<RgbaImage>>, GridFallback) {
    let mut fallback = GridFallback::default();
    let expected_rows = expected_rows.max(1);

    let bands = detect_spans_axis(img, Axis::Y);
    let bands: Vec<Span> = if bands.len() as u32 == expected_rows {
        bands
    } else {
        fallback.uniform_rows = true;
        uniform_spans(img.height(), expected_rows)
    };

    let mut rows = Vec::with_capacity(bands.len());
    for &(y0, y1) in &bands {
        let band = image::imageops::crop_imm(img, 0, y0, img.width(), y1 - y0).to_image();
        let (spans, uniform) = slice_strip(&band, expected_cols);
        if uniform {
            fallback.uniform_cols = true;
        }
        rows.push(cut_frames(&band, &spans));
    }
    (rows, fallback)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    /// 造一条 N 帧横条：每帧 40px 宽方块，帧间空白。
    fn strip(frames: u32) -> RgbaImage {
        let cell = 60u32;
        let mut img = RgbaImage::new(cell * frames, 80);
        for f in 0..frames {
            let x0 = f * cell + 10;
            for y in 20..70 {
                for x in x0..x0 + 40 {
                    img.put_pixel(x, y, Rgba([200, 100, 50, 255]));
                }
            }
        }
        img
    }

    /// 造 R 行 × C 列 网格：每格一个方块，可选白描边、可选行倾斜。
    fn grid(rows: u32, cols: u32, white_outline: bool, tilt: bool) -> RgbaImage {
        let (cw, ch) = (70u32, 90u32);
        let mut img = RgbaImage::new(cw * cols + 40, ch * rows + 40);
        for r in 0..rows {
            for c in 0..cols {
                // 轻微行倾斜：每列在 y 上偏移 0–4px
                let dy = if tilt {
                    (c * 4 / cols.max(1)) as i32
                } else {
                    0
                };
                let x0 = 20 + c * cw + 14;
                let y0 = (20 + r * ch + 20) as i32 + dy;
                for y in y0..y0 + 46 {
                    for x in x0..x0 + 40 {
                        if y >= 0 && (y as u32) < img.height() && x < img.width() {
                            let border = y < y0 + 3 || y >= y0 + 43 || x < x0 + 3 || x >= x0 + 37;
                            let px = if white_outline && border {
                                Rgba([255, 255, 255, 255])
                            } else {
                                Rgba([180, 90 + (r * 30) as u8, 60, 255])
                            };
                            img.put_pixel(x, y as u32, px);
                        }
                    }
                }
            }
        }
        img
    }

    #[test]
    fn auto_detects_frame_count() {
        for n in [1u32, 4, 6, 8] {
            let img = strip(n);
            let spans = detect_spans(&img);
            assert_eq!(spans.len() as u32, n, "应检出 {n} 帧");
        }
    }

    #[test]
    fn forced_mismatch_falls_back_to_uniform() {
        let img = strip(4);
        let (spans, fallback) = slice_strip(&img, Some(6));
        assert!(fallback);
        assert_eq!(spans.len(), 6);
    }

    #[test]
    fn forced_match_keeps_auto() {
        let img = strip(4);
        let (spans, fallback) = slice_strip(&img, Some(4));
        assert!(!fallback);
        assert_eq!(spans.len(), 4);
    }

    #[test]
    fn cut_frames_have_content() {
        let img = strip(3);
        let (spans, _) = slice_strip(&img, None);
        let frames = cut_frames(&img, &spans);
        assert_eq!(frames.len(), 3);
        for f in &frames {
            assert!(f.pixels().any(|p| p.0[3] > 0));
        }
    }

    #[test]
    fn grid_2x6_2x5_3x4_all_slice_correctly() {
        for (r, c) in [(2u32, 6u32), (2, 5), (3, 4)] {
            let img = grid(r, c, false, false);
            let (rows, fb) = slice_grid(&img, r, Some(c));
            assert_eq!(rows.len() as u32, r, "{r}×{c} 行数");
            assert!(!fb.uniform_rows, "{r}×{c} 不应行回退");
            assert!(!fb.uniform_cols, "{r}×{c} 不应列回退");
            for row in &rows {
                assert_eq!(row.len() as u32, c, "{r}×{c} 每行帧数");
            }
        }
    }

    #[test]
    fn grid_with_white_outline_slices_correctly() {
        let img = grid(2, 6, true, false);
        let (rows, fb) = slice_grid(&img, 2, Some(6));
        assert!(
            !fb.uniform_rows && !fb.uniform_cols,
            "白描边应增强而非破坏切分"
        );
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].len(), 6);
    }

    #[test]
    fn grid_with_slight_tilt_still_slices() {
        let img = grid(2, 6, false, true);
        let (rows, _) = slice_grid(&img, 2, Some(6));
        assert_eq!(rows.len(), 2);
        for row in &rows {
            assert_eq!(row.len(), 6, "轻微倾斜样本仍应正确切分");
        }
    }

    #[test]
    fn grid_row_mismatch_falls_back_uniform_rows() {
        // 声明 3 行但实际只有 2 行内容 → 等高回退出 3 行
        let img = grid(2, 4, false, false);
        let (rows, fb) = slice_grid(&img, 3, Some(4));
        assert!(fb.uniform_rows);
        assert_eq!(rows.len(), 3);
    }
}
