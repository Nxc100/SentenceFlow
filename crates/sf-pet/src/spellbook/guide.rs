//! 布局引导图生成器：作为第二张图上传给生成平台。
//! v2（改进方案 §5.3）：支持 R 行 × C 列网格版（供纯文字版式未达标的平台使用）；
//! 单行版即 rows=1 的特例。帧号用程序化七段数字绘制，零字体依赖。

use image::{Rgba, RgbaImage};

use crate::petkit::{CELL_H, CELL_W};

const SCALE: u32 = 2; // 输出 2x，交给 AI 看得更清
const GREEN: Rgba<u8> = Rgba([0, 255, 0, 255]);
const WHITE: Rgba<u8> = Rgba([255, 255, 255, 255]);
const DARK: Rgba<u8> = Rgba([20, 60, 20, 255]);

fn fill_rect(img: &mut RgbaImage, x: i64, y: i64, w: u32, h: u32, color: Rgba<u8>) {
    let (iw, ih) = img.dimensions();
    for dy in 0..h as i64 {
        for dx in 0..w as i64 {
            let (px, py) = (x + dx, y + dy);
            if px >= 0 && py >= 0 && (px as u32) < iw && (py as u32) < ih {
                img.put_pixel(px as u32, py as u32, color);
            }
        }
    }
}

/// 七段数字：段布局 A(上) B(右上) C(右下) D(下) E(左下) F(左上) G(中)。
fn draw_digit(img: &mut RgbaImage, digit: u32, x: i64, y: i64, size: u32, color: Rgba<u8>) {
    let segs: [bool; 7] = match digit {
        0 => [true, true, true, true, true, true, false],
        1 => [false, true, true, false, false, false, false],
        2 => [true, true, false, true, true, false, true],
        3 => [true, true, true, true, false, false, true],
        4 => [false, true, true, false, false, true, true],
        5 => [true, false, true, true, false, true, true],
        6 => [true, false, true, true, true, true, true],
        7 => [true, true, true, false, false, false, false],
        8 => [true, true, true, true, true, true, true],
        _ => [true, true, true, true, false, true, true], // 9
    };
    let t = (size / 5).max(3);
    let w = size;
    let h = size * 2;
    let half = h / 2;
    let rects: [(i64, i64, u32, u32); 7] = [
        (0, 0, w, t),                           // A
        ((w - t) as i64, 0, t, half),           // B
        ((w - t) as i64, half as i64, t, half), // C
        (0, (h - t) as i64, w, t),              // D
        (0, half as i64, t, half),              // E
        (0, 0, t, half),                        // F
        (0, (half - t / 2) as i64, w, t),       // G
    ];
    for (i, on) in segs.iter().enumerate() {
        if *on {
            let (dx, dy, rw, rh) = rects[i];
            fill_rect(img, x + dx, y + dy, rw, rh, color);
        }
    }
}

/// 两位数（1–99）：个位/十位并排。
fn draw_number(img: &mut RgbaImage, n: u32, x: i64, y: i64, size: u32, color: Rgba<u8>) {
    if n >= 10 {
        draw_digit(img, n / 10, x, y, size, color);
        draw_digit(img, n % 10, x + size as i64 + 8, y, size, color);
    } else {
        draw_digit(img, n, x, y, size, color);
    }
}

/// 生成 R 行 × C 列的网格布局引导图（行主序编号）。
pub fn generate_grid_guide(rows: u32, cols: u32) -> RgbaImage {
    let rows = rows.clamp(1, 3);
    let cols = cols.clamp(1, 8);
    let cw = CELL_W * SCALE;
    let ch = CELL_H * SCALE;
    let border = 6u32;
    let w = cw * cols + border * (cols + 1);
    let h = ch * rows + border * (rows + 1);
    let mut img = RgbaImage::from_pixel(w, h, GREEN);

    // 白色格线（横 + 纵）
    for r in 0..=rows {
        let y = (r * (ch + border)) as i64;
        fill_rect(&mut img, 0, y, w, border, WHITE);
    }
    for c in 0..=cols {
        let x = (c * (cw + border)) as i64;
        fill_rect(&mut img, x, 0, border, h, WHITE);
    }

    for r in 0..rows {
        for c in 0..cols {
            let cell_x = (border + c * (cw + border)) as i64;
            let cell_y = (border + r * (ch + border)) as i64;
            let number = r * cols + c + 1;

            // 帧号（左上角，带深色衬底提升对比）
            let ds = 42u32;
            draw_number(&mut img, number, cell_x + 26, cell_y + 24, ds, DARK);
            draw_number(&mut img, number, cell_x + 22, cell_y + 20, ds, WHITE);

            // 中心十字准星（角色应居中）
            let cx = cell_x + cw as i64 / 2;
            let cy = cell_y + ch as i64 / 2;
            fill_rect(&mut img, cx - 30, cy - 2, 60, 4, WHITE);
            fill_rect(&mut img, cx - 2, cy - 30, 4, 60, WHITE);

            // 底部基线（虚线）：角色脚底应站在这条线上
            let base_y = cell_y + ch as i64 - 20 * SCALE as i64;
            let mut x = cell_x + 10;
            while x < cell_x + cw as i64 - 10 {
                fill_rect(&mut img, x, base_y, 16, 4, WHITE);
                x += 28;
            }
        }
    }
    img
}

/// 单行版（rows=1 特例的便捷别名）。
#[allow(dead_code)]
pub fn generate_guide(frames: u32) -> RgbaImage {
    generate_grid_guide(1, frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_row_dimensions() {
        let img = generate_guide(6);
        assert_eq!(img.width(), 192 * 2 * 6 + 6 * 7);
        assert_eq!(img.height(), 208 * 2 + 12);
    }

    #[test]
    fn grid_2x6_dimensions() {
        let img = generate_grid_guide(2, 6);
        assert_eq!(img.width(), 192 * 2 * 6 + 6 * 7);
        assert_eq!(img.height(), 208 * 2 * 2 + 6 * 3);
    }

    #[test]
    fn guide_is_green_dominated() {
        for img in [generate_guide(4), generate_grid_guide(2, 5)] {
            let green = img.pixels().filter(|p| p.0 == [0, 255, 0, 255]).count();
            assert!(green as f32 / (img.width() * img.height()) as f32 > 0.75);
        }
    }

    #[test]
    fn params_clamped() {
        assert!(generate_grid_guide(0, 0).width() > 0);
        assert_eq!(
            generate_grid_guide(9, 99).height(),
            generate_grid_guide(3, 8).height()
        );
    }
}
