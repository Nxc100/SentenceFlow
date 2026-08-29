//! 背景移除：绿幕色键（HSV 容差 + 羽化 + 去溢色）与纯色/杂色背景的边界泛洪兜底。

use image::{Rgba, RgbaImage};

use super::rgb_to_hsv;

/// 背景类型探测结果。
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Background {
    /// 已带透明通道
    Transparent,
    /// 绿幕（走色键）
    Chroma,
    /// 其他近似纯色背景（走边界泛洪）
    Solid,
    /// 复杂背景（照片等，泛洪尽力而为）
    Busy,
}

/// 探测背景：先看 alpha，再采样边界环判断纯色/绿幕/复杂。
pub fn detect_background(img: &RgbaImage) -> Background {
    let (w, h) = img.dimensions();
    let total = (w * h) as f32;
    let transparent = img.pixels().filter(|p| p.0[3] < 16).count() as f32;
    if transparent / total > 0.01 {
        return Background::Transparent;
    }

    let ring = border_ring(img, 2);
    let n = ring.len().max(1) as f32;
    let (mut mr, mut mg, mut mb) = (0f32, 0f32, 0f32);
    for p in &ring {
        mr += p[0] as f32;
        mg += p[1] as f32;
        mb += p[2] as f32;
    }
    let (mr, mg, mb) = (mr / n, mg / n, mb / n);
    let mut var = 0f32;
    for p in &ring {
        var += (p[0] as f32 - mr).powi(2) + (p[1] as f32 - mg).powi(2) + (p[2] as f32 - mb).powi(2);
    }
    let std = (var / n).sqrt();

    let (hue, sat, val) = rgb_to_hsv(mr as u8, mg as u8, mb as u8);
    if (75.0..=165.0).contains(&hue) && sat > 0.35 && val > 0.25 {
        return Background::Chroma;
    }
    if std < 28.0 {
        return Background::Solid;
    }
    Background::Busy
}

fn border_ring(img: &RgbaImage, thickness: u32) -> Vec<[u8; 4]> {
    let (w, h) = img.dimensions();
    let t = thickness.min(w / 2).min(h / 2).max(1);
    let mut out = Vec::new();
    for y in 0..h {
        for x in 0..w {
            if x < t || y < t || x >= w - t || y >= h - t {
                out.push(img.get_pixel(x, y).0);
            }
        }
    }
    out
}

/// 容差 0..=100 映射为色键参数。
struct KeyParams {
    hue_win: f32, // 色相半窗
    sat_min: f32, // 低于该饱和度不视为幕色
    val_min: f32,
    feather: f32, // 羽化带宽（相对 hue_win）
}

fn key_params(tolerance: f32) -> KeyParams {
    let t = tolerance.clamp(0.0, 100.0) / 100.0;
    KeyParams {
        hue_win: 18.0 + 42.0 * t,
        sat_min: (0.42 - 0.30 * t).max(0.08),
        val_min: (0.28 - 0.20 * t).max(0.06),
        feather: 8.0,
    }
}

/// 绿幕色键：命中→透明（RGB 清零，杜绝残留），边缘羽化，剩余像素去绿色溢色。
pub fn key_out_green(img: &RgbaImage, tolerance: f32) -> RgbaImage {
    let p = key_params(tolerance);
    let key_hue = 120.0f32;
    let mut out = img.clone();

    for px in out.pixels_mut() {
        let [r, g, b, a] = px.0;
        if a < 8 {
            *px = Rgba([0, 0, 0, 0]);
            continue;
        }
        let (h, s, v) = rgb_to_hsv(r, g, b);
        if s < p.sat_min || v < p.val_min {
            continue;
        }
        let dh = (h - key_hue).abs().min(360.0 - (h - key_hue).abs());
        if dh <= p.hue_win {
            *px = Rgba([0, 0, 0, 0]);
        } else if dh <= p.hue_win + p.feather {
            // 羽化带：alpha 线性过渡，同时按比例压暗以避免绿边
            let k = (dh - p.hue_win) / p.feather; // 0..1
            let na = (a as f32 * k) as u8;
            if na < 8 {
                *px = Rgba([0, 0, 0, 0]);
            } else {
                *px = Rgba([r, g.min(r.max(b)), b, na]);
            }
        }
    }

    despill_green(&mut out);
    out
}

/// 去溢色：对残余偏绿像素把 G 压到 max(R,B) 附近。
fn despill_green(img: &mut RgbaImage) {
    for px in img.pixels_mut() {
        let [r, g, b, a] = px.0;
        if a == 0 {
            continue;
        }
        let cap = r.max(b).saturating_add(12);
        if g > cap {
            let (h, s, _) = rgb_to_hsv(r, g, b);
            if (60.0..=180.0).contains(&h) && s > 0.25 {
                px.0[1] = cap;
            }
        }
    }
}

/// 边界泛洪移除（L0 任意图兜底）：从四边出发，删除与边界均色相近的连通区域。
pub fn flood_remove_background(img: &RgbaImage, tolerance: f32) -> RgbaImage {
    let (w, h) = img.dimensions();
    let mut out = img.clone();
    if w == 0 || h == 0 {
        return out;
    }
    let ring = border_ring(img, 2);
    let n = ring.len().max(1) as f32;
    let (mut mr, mut mg, mut mb) = (0f32, 0f32, 0f32);
    for p in &ring {
        mr += p[0] as f32;
        mg += p[1] as f32;
        mb += p[2] as f32;
    }
    let bg = [(mr / n) as i32, (mg / n) as i32, (mb / n) as i32];
    // 容差 0..=100 → 颜色距离阈值
    let thr = 28.0 + 1.6 * tolerance.clamp(0.0, 100.0);
    let thr2 = thr * thr;

    let close = |p: &[u8; 4]| -> bool {
        let dr = p[0] as i32 - bg[0];
        let dg = p[1] as i32 - bg[1];
        let db = p[2] as i32 - bg[2];
        ((dr * dr + dg * dg + db * db) as f32) < thr2
    };

    let idx = |x: u32, y: u32| (y * w + x) as usize;
    let mut visited = vec![false; (w * h) as usize];
    let mut queue = std::collections::VecDeque::new();
    for x in 0..w {
        for y in [0, h - 1] {
            if !visited[idx(x, y)] && close(&img.get_pixel(x, y).0) {
                visited[idx(x, y)] = true;
                queue.push_back((x, y));
            }
        }
    }
    for y in 0..h {
        for x in [0, w - 1] {
            if !visited[idx(x, y)] && close(&img.get_pixel(x, y).0) {
                visited[idx(x, y)] = true;
                queue.push_back((x, y));
            }
        }
    }
    while let Some((x, y)) = queue.pop_front() {
        out.put_pixel(x, y, Rgba([0, 0, 0, 0]));
        let neighbors = [
            (x.wrapping_sub(1), y),
            (x + 1, y),
            (x, y.wrapping_sub(1)),
            (x, y + 1),
        ];
        for (nx, ny) in neighbors {
            if nx < w && ny < h && !visited[idx(nx, ny)] && close(&img.get_pixel(nx, ny).0) {
                visited[idx(nx, ny)] = true;
                queue.push_back((nx, ny));
            }
        }
    }
    out
}

/// 归一化已透明图：把 alpha=0 的像素 RGB 清零（消除“非零 RGB 残留”）。
pub fn normalize_transparent(img: &mut RgbaImage) {
    for px in img.pixels_mut() {
        if px.0[3] == 0 {
            *px = Rgba([0, 0, 0, 0]);
        }
    }
}

// ---------------------------------------------------------------- 改进方案新增

/// 素材来源（决定自动容差基线，改进方案 M3）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SourceKind {
    /// 静态图片导入
    Image,
    /// 视频抽帧（压缩噪声大，容差 +15）
    Video,
}

/// 容差自动估计（M3）：按背景类型与幕色纯度给出基线，滑杆收进「高级」。
/// 绿幕越不纯（边界环色彩方差越大）容差越高；视频源整体 +15 吸收压缩噪声。
pub fn auto_tolerance(img: &RgbaImage, source: SourceKind) -> f32 {
    let base = match detect_background(img) {
        Background::Transparent => 0.0,
        Background::Chroma => {
            let ring = border_ring(img, 2);
            let n = ring.len().max(1) as f32;
            let (mut mr, mut mg, mut mb) = (0f32, 0f32, 0f32);
            for p in &ring {
                mr += p[0] as f32;
                mg += p[1] as f32;
                mb += p[2] as f32;
            }
            let (mr, mg, mb) = (mr / n, mg / n, mb / n);
            let mut var = 0f32;
            for p in &ring {
                var += (p[0] as f32 - mr).powi(2)
                    + (p[1] as f32 - mg).powi(2)
                    + (p[2] as f32 - mb).powi(2);
            }
            let std = (var / n).sqrt();
            // 纯净绿幕 35，随杂色程度线性上调至 55
            (35.0 + std * 0.8).min(55.0)
        }
        Background::Solid => 45.0,
        Background::Busy => 45.0,
    };
    match source {
        SourceKind::Image => base,
        SourceKind::Video => (base + 15.0).min(80.0),
    }
}

/// 边缘绿边去污（M7）：对半透明羽化边像素做绿色通道抑制。
/// 与 `despill_green`（针对不透明溢色）互补，对视频源尤其关键。
pub fn despill_edges(img: &mut RgbaImage) {
    for px in img.pixels_mut() {
        let [r, g, b, a] = px.0;
        if a == 0 || a == 255 {
            continue;
        }
        let cap = ((r as u16 + b as u16) / 2) as u8;
        if g > cap.saturating_add(8) {
            px.0[1] = cap.saturating_add(8);
        }
    }
}

/// 角标 ROI（相对整图的比例坐标，来自 spellbook data.json，可热更）。
#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct CornerRoi {
    /// "top-left" | "top-right" | "bottom-left" | "bottom-right"
    pub corner: String,
    /// ROI 宽（相对整图宽的比例，如 0.14）
    pub w: f32,
    /// ROI 高（相对整图高的比例，如 0.09）
    pub h: f32,
}

impl CornerRoi {
    fn rect(&self, w: u32, h: u32) -> (u32, u32, u32, u32) {
        let rw = ((w as f32 * self.w).round() as u32).clamp(1, w);
        let rh = ((h as f32 * self.h).round() as u32).clamp(1, h);
        match self.corner.as_str() {
            "top-left" => (0, 0, rw, rh),
            "top-right" => (w - rw, 0, rw, rh),
            "bottom-left" => (0, h - rh, rw, rh),
            _ => (w - rw, h - rh, rw, rh), // bottom-right（默认）
        }
    }
}

/// 默认四角 ROI（宽 14% × 高 9%，方案 §5.4）。
pub fn default_corner_rois() -> Vec<CornerRoi> {
    ["top-left", "top-right", "bottom-left", "bottom-right"]
        .iter()
        .map(|c| CornerRoi {
            corner: (*c).to_string(),
            w: 0.14,
            h: 0.09,
        })
        .collect()
}

/// 角标区清除结果。
pub struct CornerCleanReport {
    /// 被清除的像素数
    pub cleared: u32,
    /// 主体（最大连通域）与某个 ROI 相交 → 触发"角色距角落过近"软性警告
    pub character_touches_corner: bool,
}

/// 平台角标区清除（§5.4 第 2 层，在抠色**之后**调用）：
/// 找最大不透明连通域视为角色主体；各 ROI 内**不属于主体**的内容判定为平台角标，
/// 置为透明；主体包围盒与 ROI 相交则仅告警、不动像素（避免误伤角色）。
pub fn clear_corner_watermarks(img: &mut RgbaImage, rois: &[CornerRoi]) -> CornerCleanReport {
    let (w, h) = img.dimensions();
    let mut report = CornerCleanReport {
        cleared: 0,
        character_touches_corner: false,
    };
    if w == 0 || h == 0 || rois.is_empty() {
        return report;
    }

    // 连通域标记（4 邻接），找最大者作为主体
    let idx = |x: u32, y: u32| (y * w + x) as usize;
    let mut label = vec![0u32; (w * h) as usize]; // 0 = 未标记/透明
    let mut sizes: Vec<u32> = vec![0]; // sizes[label]
    let mut next = 1u32;
    for sy in 0..h {
        for sx in 0..w {
            if label[idx(sx, sy)] != 0 || img.get_pixel(sx, sy).0[3] < super::ALPHA_SOLID {
                continue;
            }
            let mut queue = std::collections::VecDeque::new();
            label[idx(sx, sy)] = next;
            queue.push_back((sx, sy));
            let mut size = 0u32;
            while let Some((x, y)) = queue.pop_front() {
                size += 1;
                let neighbors = [
                    (x.wrapping_sub(1), y),
                    (x + 1, y),
                    (x, y.wrapping_sub(1)),
                    (x, y + 1),
                ];
                for (nx, ny) in neighbors {
                    if nx < w
                        && ny < h
                        && label[idx(nx, ny)] == 0
                        && img.get_pixel(nx, ny).0[3] >= super::ALPHA_SOLID
                    {
                        label[idx(nx, ny)] = next;
                        queue.push_back((nx, ny));
                    }
                }
            }
            sizes.push(size);
            next += 1;
        }
    }
    let main_label = (1..sizes.len()).max_by_key(|&l| sizes[l]).unwrap_or(0) as u32;
    if main_label == 0 {
        return report;
    }

    for roi in rois {
        let (rx, ry, rw, rh) = roi.rect(w, h);
        for y in ry..ry + rh {
            for x in rx..rx + rw {
                let l = label[idx(x, y)];
                if l == 0 {
                    continue;
                }
                if l == main_label {
                    report.character_touches_corner = true;
                } else {
                    img.put_pixel(x, y, Rgba([0, 0, 0, 0]));
                    report.cleared += 1;
                }
            }
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn green_bg_with_red_square(w: u32, h: u32) -> RgbaImage {
        let mut img = RgbaImage::from_pixel(w, h, Rgba([0, 255, 0, 255]));
        for y in h / 4..h * 3 / 4 {
            for x in w / 4..w * 3 / 4 {
                img.put_pixel(x, y, Rgba([200, 40, 40, 255]));
            }
        }
        img
    }

    #[test]
    fn detects_chroma_background() {
        let img = green_bg_with_red_square(64, 64);
        assert_eq!(detect_background(&img), Background::Chroma);
    }

    #[test]
    fn detects_transparent() {
        let mut img = RgbaImage::new(32, 32);
        img.put_pixel(16, 16, Rgba([255, 0, 0, 255]));
        assert_eq!(detect_background(&img), Background::Transparent);
    }

    #[test]
    fn detects_solid_non_green() {
        let mut img = RgbaImage::from_pixel(64, 64, Rgba([240, 240, 238, 255]));
        for y in 20..44 {
            for x in 20..44 {
                img.put_pixel(x, y, Rgba([30, 60, 200, 255]));
            }
        }
        assert_eq!(detect_background(&img), Background::Solid);
    }

    #[test]
    fn key_out_removes_green_keeps_subject() {
        let img = green_bg_with_red_square(64, 64);
        let out = key_out_green(&img, 40.0);
        assert_eq!(out.get_pixel(1, 1).0[3], 0, "绿幕应被抠除");
        assert_eq!(out.get_pixel(1, 1).0[0], 0, "透明像素 RGB 必须清零");
        assert_eq!(out.get_pixel(32, 32).0[3], 255, "主体应保留");
    }

    #[test]
    fn flood_removes_solid_background() {
        let mut img = RgbaImage::from_pixel(64, 64, Rgba([240, 240, 238, 255]));
        for y in 20..44 {
            for x in 20..44 {
                img.put_pixel(x, y, Rgba([30, 60, 200, 255]));
            }
        }
        let out = flood_remove_background(&img, 30.0);
        assert_eq!(out.get_pixel(2, 2).0[3], 0);
        assert_eq!(out.get_pixel(32, 32).0[3], 255);
    }

    #[test]
    fn auto_tolerance_scales_by_source_and_purity() {
        let clean = green_bg_with_red_square(64, 64);
        let t_img = auto_tolerance(&clean, SourceKind::Image);
        let t_vid = auto_tolerance(&clean, SourceKind::Video);
        assert!(
            (30.0..=40.0).contains(&t_img),
            "纯净绿幕基线应约 35，实际 {t_img}"
        );
        assert!((t_vid - t_img - 15.0).abs() < 0.01, "视频源 +15");
        // 杂色绿幕容差应更高
        let mut noisy = green_bg_with_red_square(64, 64);
        for y in 0..64u32 {
            for x in 0..64u32 {
                if noisy.get_pixel(x, y).0 == [0, 255, 0, 255] && (x + y) % 3 == 0 {
                    noisy.put_pixel(x, y, Rgba([40, 210, 60, 255]));
                }
            }
        }
        assert!(auto_tolerance(&noisy, SourceKind::Image) > t_img);
    }

    #[test]
    fn despill_edges_suppresses_green_fringe() {
        let mut img = RgbaImage::new(8, 8);
        img.put_pixel(2, 2, Rgba([60, 220, 70, 128])); // 半透明绿边
        img.put_pixel(3, 3, Rgba([60, 220, 70, 255])); // 不透明像素不动
        despill_edges(&mut img);
        assert!(img.get_pixel(2, 2).0[1] <= 73, "羽化边绿色分量应被压制");
        assert_eq!(img.get_pixel(3, 3).0[1], 220, "不透明像素不受影响");
    }

    #[test]
    fn corner_watermark_cleared_character_kept() {
        // 中央大主体 + 右下角一块"角标"
        let mut img = RgbaImage::new(200, 100);
        for y in 30..70 {
            for x in 70..130 {
                img.put_pixel(x, y, Rgba([200, 80, 60, 255]));
            }
        }
        for y in 92..98 {
            for x in 175..197 {
                img.put_pixel(x, y, Rgba([120, 120, 120, 255]));
            }
        }
        let report = clear_corner_watermarks(&mut img, &default_corner_rois());
        assert!(report.cleared > 0, "角标应被清除");
        assert!(!report.character_touches_corner);
        assert_eq!(img.get_pixel(180, 95).0[3], 0, "角标像素已透明");
        assert_eq!(img.get_pixel(100, 50).0[3], 255, "主体完好");
    }

    #[test]
    fn character_touching_corner_warns_without_erasing() {
        // 主体一直延伸到右下角 ROI 内
        let mut img = RgbaImage::new(200, 100);
        for y in 20..100 {
            for x in 60..200 {
                img.put_pixel(x, y, Rgba([200, 80, 60, 255]));
            }
        }
        let report = clear_corner_watermarks(&mut img, &default_corner_rois());
        assert!(report.character_touches_corner, "应触发压角警告");
        assert_eq!(img.get_pixel(198, 98).0[3], 255, "主体像素不应被误删");
    }

    #[test]
    fn flood_does_not_leak_into_enclosed_subject() {
        // 主体中央有一块与背景同色的"眼睛"，因被主体包围不应被移除
        let mut img = RgbaImage::from_pixel(64, 64, Rgba([250, 250, 250, 255]));
        for y in 16..48 {
            for x in 16..48 {
                img.put_pixel(x, y, Rgba([40, 40, 40, 255]));
            }
        }
        for y in 30..34 {
            for x in 30..34 {
                img.put_pixel(x, y, Rgba([250, 250, 250, 255]));
            }
        }
        let out = flood_remove_background(&img, 30.0);
        assert_eq!(
            out.get_pixel(32, 32).0[3],
            255,
            "被包围的同色区域不应被泛洪"
        );
    }
}
