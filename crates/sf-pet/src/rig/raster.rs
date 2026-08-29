//! 阶段六·软光栅（§3.6，§1.1）：逐三角仿射纹理绘制 → RGBA 缓冲。手写零依赖。
//! 纹理即源图，UV = rest 顶点坐标；目标三角为 ARAP 形变后顶点。重心插值 + 双线性采样 +
//! source-over 合成。输出直接喂入现有 layout::fit_into_cell / compose_sheet（零 IPC）。

use image::{Rgba, RgbaImage};

use super::V2;
use super::mesh::Mesh;

/// 把形变网格光栅化到 out_w×out_h 缓冲。deformed 与 rest 一一对应；offset 平移到画布内。
pub fn rasterize(
    source: &RgbaImage,
    mesh: &Mesh,
    deformed: &[V2],
    out_w: u32,
    out_h: u32,
    offset: V2,
) -> RgbaImage {
    let mut out = RgbaImage::from_pixel(out_w, out_h, Rgba([0, 0, 0, 0]));
    for tri in &mesh.tris {
        let r = [mesh.verts[tri[0]], mesh.verts[tri[1]], mesh.verts[tri[2]]]; // rest / UV
        let d = [
            deformed[tri[0]].add(offset),
            deformed[tri[1]].add(offset),
            deformed[tri[2]].add(offset),
        ];
        draw_tri(&mut out, source, &r, &d);
    }
    out
}

fn draw_tri(out: &mut RgbaImage, src: &RgbaImage, r: &[V2; 3], d: &[V2; 3]) {
    let (ow, oh) = out.dimensions();
    let min_x = d
        .iter()
        .map(|p| p.x)
        .fold(f64::INFINITY, f64::min)
        .floor()
        .max(0.0) as u32;
    let max_x = d
        .iter()
        .map(|p| p.x)
        .fold(f64::NEG_INFINITY, f64::max)
        .ceil()
        .min((ow - 1) as f64) as u32;
    let min_y = d
        .iter()
        .map(|p| p.y)
        .fold(f64::INFINITY, f64::min)
        .floor()
        .max(0.0) as u32;
    let max_y = d
        .iter()
        .map(|p| p.y)
        .fold(f64::NEG_INFINITY, f64::max)
        .ceil()
        .min((oh - 1) as f64) as u32;
    if min_x > max_x || min_y > max_y {
        return;
    }
    let denom = (d[1].y - d[2].y) * (d[0].x - d[2].x) + (d[2].x - d[1].x) * (d[0].y - d[2].y);
    if denom.abs() < 1e-9 {
        return; // 退化三角
    }
    let inv = 1.0 / denom;
    for py in min_y..=max_y {
        for px in min_x..=max_x {
            let (fx, fy) = (px as f64 + 0.5, py as f64 + 0.5);
            let w0 = ((d[1].y - d[2].y) * (fx - d[2].x) + (d[2].x - d[1].x) * (fy - d[2].y)) * inv;
            let w1 = ((d[2].y - d[0].y) * (fx - d[2].x) + (d[0].x - d[2].x) * (fy - d[2].y)) * inv;
            let w2 = 1.0 - w0 - w1;
            let eps = -1e-4;
            if w0 < eps || w1 < eps || w2 < eps {
                continue;
            }
            // 源坐标 = 重心插值 rest
            let sx = w0 * r[0].x + w1 * r[1].x + w2 * r[2].x;
            let sy = w0 * r[0].y + w1 * r[1].y + w2 * r[2].y;
            let sample = bilinear(src, sx, sy);
            if sample[3] == 0 {
                continue;
            }
            let dst = out.get_pixel_mut(px, py);
            *dst = over(sample, *dst);
        }
    }
}

/// 源图双线性采样（越界钳制）。
fn bilinear(src: &RgbaImage, x: f64, y: f64) -> [u8; 4] {
    let (w, h) = src.dimensions();
    if w == 0 || h == 0 {
        return [0, 0, 0, 0];
    }
    let x = x.clamp(0.0, (w - 1) as f64);
    let y = y.clamp(0.0, (h - 1) as f64);
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(w - 1);
    let y1 = (y0 + 1).min(h - 1);
    let tx = x - x0 as f64;
    let ty = y - y0 as f64;
    let p00 = src.get_pixel(x0, y0).0;
    let p10 = src.get_pixel(x1, y0).0;
    let p01 = src.get_pixel(x0, y1).0;
    let p11 = src.get_pixel(x1, y1).0;
    let mut out = [0u8; 4];
    for c in 0..4 {
        let top = p00[c] as f64 * (1.0 - tx) + p10[c] as f64 * tx;
        let bot = p01[c] as f64 * (1.0 - tx) + p11[c] as f64 * tx;
        out[c] = (top * (1.0 - ty) + bot * ty).round().clamp(0.0, 255.0) as u8;
    }
    out
}

/// source-over 合成（直通 alpha）。
fn over(s: [u8; 4], d: Rgba<u8>) -> Rgba<u8> {
    let sa = s[3] as f64 / 255.0;
    if sa >= 0.999 {
        return Rgba(s);
    }
    let da = d.0[3] as f64 / 255.0;
    let oa = sa + da * (1.0 - sa);
    if oa < 1e-6 {
        return Rgba([0, 0, 0, 0]);
    }
    let mut out = [0u8; 4];
    for c in 0..3 {
        let v = (s[c] as f64 * sa + d.0[c] as f64 * da * (1.0 - sa)) / oa;
        out[c] = v.round().clamp(0.0, 255.0) as u8;
    }
    out[3] = (oa * 255.0).round().clamp(0.0, 255.0) as u8;
    Rgba(out)
}

#[cfg(test)]
mod tests {
    use super::super::mesh;
    use super::*;

    #[test]
    fn identity_reproduces_source() {
        // 纯色方块，网格覆盖，identity 形变 → 输出内部应复现源色。
        let mut src = RgbaImage::from_pixel(60, 60, Rgba([0, 0, 0, 0]));
        for y in 10..50 {
            for x in 10..50 {
                src.put_pixel(x, y, Rgba([200, 120, 60, 255]));
            }
        }
        let mask: Vec<bool> = (0..60 * 60)
            .map(|i| {
                let (x, y) = (i % 60, i / 60);
                (10..50).contains(&x) && (10..50).contains(&y)
            })
            .collect();
        let m = mesh::build(&mask, 60, 60, 300);
        let out = rasterize(&src, &m, &m.verts, 60, 60, V2::new(0.0, 0.0));
        // 网格内部像素应≈源色
        let p = out.get_pixel(30, 30).0;
        assert!(p[3] > 200, "center alpha {}", p[3]);
        assert!((p[0] as i32 - 200).abs() < 20 && (p[1] as i32 - 120).abs() < 20);
    }

    #[test]
    fn translation_shifts_content() {
        let mut src = RgbaImage::from_pixel(60, 60, Rgba([0, 0, 0, 0]));
        for y in 10..50 {
            for x in 10..50 {
                src.put_pixel(x, y, Rgba([50, 200, 90, 255]));
            }
        }
        let mask: Vec<bool> = (0..60 * 60)
            .map(|i| {
                let (x, y) = (i % 60, i / 60);
                (10..50).contains(&x) && (10..50).contains(&y)
            })
            .collect();
        let m = mesh::build(&mask, 60, 60, 300);
        let out = rasterize(&src, &m, &m.verts, 80, 80, V2::new(15.0, 0.0));
        // 平移后内容在 x+15 处出现
        assert!(out.get_pixel(45, 30).0[3] > 150);
    }
}
