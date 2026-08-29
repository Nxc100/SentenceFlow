//! 对齐与合成：帧 → 192×208 单元格（水平质心居中 + 底部基线），行 → 8×8 图集。
//!
//! 缩放系数在整只宠物的**全部帧**上统一计算（取各帧外接盒最大值），
//! 保证 idle 与 walk 等状态间的相对大小一致。

use std::collections::BTreeMap;

use image::{Rgba, RgbaImage, imageops};

use super::ALPHA_SOLID;
use crate::petkit::{CELL_H, CELL_W, GRID_COLS, GRID_ROWS, SHEET_H, SHEET_W, StateId};

/// 不透明像素外接盒（闭区间端点 + 尺寸），无内容返回 None。
#[derive(Clone, Copy, Debug)]
pub struct Bbox {
    pub x0: u32,
    pub y0: u32,
    pub w: u32,
    pub h: u32,
    pub centroid_x: f32,
}

pub fn bbox(img: &RgbaImage) -> Option<Bbox> {
    let (w, h) = img.dimensions();
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0u32, 0u32);
    let (mut cx_sum, mut count) = (0f64, 0u64);
    for y in 0..h {
        for x in 0..w {
            if img.get_pixel(x, y).0[3] >= ALPHA_SOLID {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
                cx_sum += x as f64;
                count += 1;
            }
        }
    }
    if count == 0 {
        return None;
    }
    Some(Bbox {
        x0,
        y0,
        w: x1 - x0 + 1,
        h: y1 - y0 + 1,
        centroid_x: (cx_sum / count as f64) as f32,
    })
}

/// 单元格内边距：水平 8px、顶部 10px、底部基线 10px。
const PAD_X: u32 = 8;
const PAD_TOP: u32 = 10;
const BASELINE: u32 = 10;

/// 计算一组帧的统一缩放系数（适配单元格可用区域）。
pub fn global_scale(frames: &[&RgbaImage]) -> f32 {
    let mut max_w = 1u32;
    let mut max_h = 1u32;
    for f in frames {
        if let Some(b) = bbox(f) {
            max_w = max_w.max(b.w);
            max_h = max_h.max(b.h);
        }
    }
    let avail_w = (CELL_W - PAD_X * 2) as f32;
    let avail_h = (CELL_H - PAD_TOP - BASELINE) as f32;
    (avail_w / max_w as f32).min(avail_h / max_h as f32)
}

/// 把一帧放入 192×208 单元格：统一缩放 → 质心水平居中 → 底部基线对齐。
pub fn fit_into_cell(frame: &RgbaImage, scale: f32) -> RgbaImage {
    let mut cell = RgbaImage::from_pixel(CELL_W, CELL_H, Rgba([0, 0, 0, 0]));
    let Some(b) = bbox(frame) else {
        return cell;
    };
    let cropped = imageops::crop_imm(frame, b.x0, b.y0, b.w, b.h).to_image();
    let nw = ((b.w as f32 * scale).round() as u32).clamp(1, CELL_W);
    let nh = ((b.h as f32 * scale).round() as u32).clamp(1, CELL_H);
    // 放大用最近邻（像素风），缩小用 Lanczos（照片抗锯齿）
    let filter = if scale >= 1.0 {
        imageops::FilterType::Nearest
    } else {
        imageops::FilterType::Lanczos3
    };
    let resized = imageops::resize(&cropped, nw, nh, filter);

    // 质心（相对裁剪区）映射到缩放后坐标，水平对齐到单元格中线
    let centroid_local = (b.centroid_x - b.x0 as f32) * scale;
    let dst_x = (CELL_W as f32 / 2.0 - centroid_local).round() as i64;
    let dst_x = dst_x.clamp(0, (CELL_W - nw.min(CELL_W)) as i64);
    let dst_y = (CELL_H - BASELINE).saturating_sub(nh) as i64;

    imageops::overlay(&mut cell, &resized, dst_x, dst_y);
    cell
}

/// 水平镜像（walk-left 通常由运行时 canvas 翻转派生；此函数供导出等需要实像素的场景）。
#[allow(dead_code)]
pub fn mirror(img: &RgbaImage) -> RgbaImage {
    imageops::flip_horizontal(img)
}

/// 合成 8×8 图集：`rows[state] = 已对齐到单元格的帧`。
pub fn compose_sheet(rows: &BTreeMap<StateId, Vec<RgbaImage>>) -> RgbaImage {
    let mut sheet = RgbaImage::from_pixel(SHEET_W, SHEET_H, Rgba([0, 0, 0, 0]));
    for (state, frames) in rows {
        let row = state.row();
        for (i, cell) in frames.iter().enumerate().take(GRID_COLS as usize) {
            debug_assert_eq!(cell.dimensions(), (CELL_W, CELL_H));
            imageops::overlay(
                &mut sheet,
                cell,
                (i as u32 * CELL_W) as i64,
                (row * CELL_H) as i64,
            );
        }
    }
    debug_assert_eq!(sheet.dimensions(), (CELL_W * GRID_COLS, CELL_H * GRID_ROWS));
    sheet
}

/// 从图集裁出某状态某帧的单元格。
pub fn cell_at(sheet: &RgbaImage, row: u32, col: u32) -> RgbaImage {
    imageops::crop_imm(sheet, col * CELL_W, row * CELL_H, CELL_W, CELL_H).to_image()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blob(w: u32, h: u32, x0: u32, y0: u32, bw: u32, bh: u32) -> RgbaImage {
        let mut img = RgbaImage::new(w, h);
        for y in y0..y0 + bh {
            for x in x0..x0 + bw {
                img.put_pixel(x, y, Rgba([120, 80, 200, 255]));
            }
        }
        img
    }

    #[test]
    fn bbox_and_centroid() {
        let img = blob(100, 100, 20, 30, 40, 20);
        let b = bbox(&img).unwrap();
        assert_eq!((b.x0, b.y0, b.w, b.h), (20, 30, 40, 20));
        assert!((b.centroid_x - 39.5).abs() < 0.6);
    }

    #[test]
    fn fit_centers_and_bottom_aligns() {
        let img = blob(400, 400, 50, 50, 100, 200);
        let s = global_scale(&[&img]);
        let cell = fit_into_cell(&img, s);
        assert_eq!(cell.dimensions(), (CELL_W, CELL_H));
        let b = bbox(&cell).unwrap();
        // 底部基线：内容底边应贴近 CELL_H - BASELINE
        assert!((b.y0 + b.h).abs_diff(CELL_H - BASELINE) <= 2);
        // 水平居中
        let cx = b.x0 as f32 + b.w as f32 / 2.0;
        assert!((cx - CELL_W as f32 / 2.0).abs() < 4.0);
    }

    #[test]
    fn oversized_frame_fits_within_cell() {
        let img = blob(1000, 300, 0, 0, 1000, 300);
        let s = global_scale(&[&img]);
        let cell = fit_into_cell(&img, s);
        let b = bbox(&cell).unwrap();
        assert!(b.w <= CELL_W - 2 * PAD_X + 2);
        assert!(b.h <= CELL_H);
    }

    #[test]
    fn compose_places_rows_at_contract_positions() {
        let mut rows = BTreeMap::new();
        let frame = fit_into_cell(&blob(200, 200, 40, 40, 80, 120), 1.0);
        rows.insert(StateId::Idle, vec![frame.clone(), frame.clone()]);
        rows.insert(StateId::Sleep, vec![frame.clone()]);
        let sheet = compose_sheet(&rows);
        assert_eq!(sheet.dimensions(), (1536, 1664));
        // idle 行（row0）第 0 帧有内容
        assert!(bbox(&cell_at(&sheet, 0, 0)).is_some());
        // sleep 行（row5）第 0 帧有内容，第 1 帧为空
        assert!(bbox(&cell_at(&sheet, 5, 0)).is_some());
        assert!(bbox(&cell_at(&sheet, 5, 1)).is_none());
        // 未使用行全空
        assert!(bbox(&cell_at(&sheet, 3, 0)).is_none());
    }

    #[test]
    fn mirror_flips() {
        let mut img = RgbaImage::new(10, 10);
        img.put_pixel(1, 5, Rgba([255, 0, 0, 255]));
        let m = mirror(&img);
        assert_eq!(m.get_pixel(8, 5).0[3], 255);
    }
}
