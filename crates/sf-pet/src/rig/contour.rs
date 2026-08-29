//! 阶段三·轮廓提取（§3.3）：alpha mask → 最大连通域边界（Moore 邻域追踪）→
//! Douglas–Peucker 简化（60–120 顶点）。手写零依赖，确定性可测。

use image::RgbaImage;

use super::V2;
use crate::imaging::ALPHA_SOLID;

/// 二值 mask + 最大连通域筛选。返回 (mask, w, h)：mask[y*w+x]=该像素属于最大连通域。
pub fn solid_mask(img: &RgbaImage, threshold: u8) -> (Vec<bool>, usize, usize) {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let mut raw = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            raw[y * w + x] = img.get_pixel(x as u32, y as u32).0[3] >= threshold;
        }
    }
    largest_component(&raw, w, h)
}

/// 4 连通 BFS 标记，只保留像素数最多的连通域。
fn largest_component(raw: &[bool], w: usize, h: usize) -> (Vec<bool>, usize, usize) {
    let mut label = vec![0u32; w * h];
    let mut best: (u32, usize) = (0, 0);
    let mut next = 1u32;
    let mut stack = Vec::new();
    for start in 0..w * h {
        if !raw[start] || label[start] != 0 {
            continue;
        }
        let id = next;
        next += 1;
        let mut count = 0usize;
        stack.push(start);
        label[start] = id;
        while let Some(i) = stack.pop() {
            count += 1;
            let (x, y) = (i % w, i / w);
            let push = |nx: usize, ny: usize, stack: &mut Vec<usize>, label: &mut [u32]| {
                let ni = ny * w + nx;
                if raw[ni] && label[ni] == 0 {
                    label[ni] = id;
                    stack.push(ni);
                }
            };
            if x > 0 {
                push(x - 1, y, &mut stack, &mut label);
            }
            if x + 1 < w {
                push(x + 1, y, &mut stack, &mut label);
            }
            if y > 0 {
                push(x, y - 1, &mut stack, &mut label);
            }
            if y + 1 < h {
                push(x, y + 1, &mut stack, &mut label);
            }
        }
        if count > best.1 {
            best = (id, count);
        }
    }
    let mask: Vec<bool> = label.iter().map(|&l| l != 0 && l == best.0).collect();
    (mask, w, h)
}

/// Moore 邻域边界追踪（Jacob 停止准则）：返回有序外边界像素环。空 mask 返回空。
pub fn trace_boundary(mask: &[bool], w: usize, h: usize) -> Vec<V2> {
    let at = |x: i32, y: i32| -> bool {
        x >= 0
            && y >= 0
            && (x as usize) < w
            && (y as usize) < h
            && mask[y as usize * w + x as usize]
    };
    // 起点：从上到下、从左到右第一个实心像素
    let mut start = None;
    'outer: for y in 0..h as i32 {
        for x in 0..w as i32 {
            if at(x, y) {
                start = Some((x, y));
                break 'outer;
            }
        }
    }
    let Some(start) = start else {
        return Vec::new();
    };
    // 单像素连通域：直接返回单点
    let count = mask.iter().filter(|&&b| b).count();
    if count == 1 {
        return vec![V2::new(start.0 as f64, start.1 as f64)];
    }

    // Moore 邻域（顺时针，从正左开始）
    const NB: [(i32, i32); 8] = [
        (-1, 0),
        (-1, -1),
        (0, -1),
        (1, -1),
        (1, 0),
        (1, 1),
        (0, 1),
        (-1, 1),
    ];
    let mut contour = Vec::new();
    let mut p = start;
    // 进入方向：来自左侧空像素
    let mut backtrack = (start.0 - 1, start.1);
    let start_backtrack = backtrack;
    let mut guard = 0usize;
    let max_iter = w * h * 8 + 16;
    loop {
        contour.push(V2::new(p.0 as f64, p.1 as f64));
        // 从 backtrack 相对 p 的方向开始，顺时针找下一个实心像素
        let bd = (backtrack.0 - p.0, backtrack.1 - p.1);
        let mut idx = NB.iter().position(|&d| d == bd).unwrap_or(0);
        let mut found = None;
        for _ in 0..8 {
            idx = (idx + 1) % 8;
            let c = (p.0 + NB[idx].0, p.1 + NB[idx].1);
            if at(c.0, c.1) {
                found = Some((c, (p.0 + NB[(idx + 7) % 8].0, p.1 + NB[(idx + 7) % 8].1)));
                break;
            }
        }
        let Some((nextp, nextback)) = found else {
            break; // 孤立像素
        };
        backtrack = nextback;
        p = nextp;
        guard += 1;
        // Jacob 停止：回到起点且进入方向一致
        if p == start && backtrack == start_backtrack {
            break;
        }
        if guard > max_iter {
            break;
        }
    }
    contour
}

/// Douglas–Peucker 折线简化（保留首尾），epsilon 为像素容差。
pub fn simplify_rdp(pts: &[V2], epsilon: f64) -> Vec<V2> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut keep = vec![false; pts.len()];
    keep[0] = true;
    *keep.last_mut().unwrap() = true;
    rdp_rec(pts, 0, pts.len() - 1, epsilon, &mut keep);
    pts.iter()
        .zip(keep.iter())
        .filter(|(_, k)| **k)
        .map(|(p, _)| *p)
        .collect()
}

// 记录的是「最远点的下标」，下标本身就是结果，不只是取值的手段。
#[allow(clippy::needless_range_loop)]
fn rdp_rec(pts: &[V2], i0: usize, i1: usize, eps: f64, keep: &mut [bool]) {
    if i1 <= i0 + 1 {
        return;
    }
    let (a, b) = (pts[i0], pts[i1]);
    let ab = b.sub(a);
    let ab_len = ab.len().max(1e-9);
    let mut max_d = 0.0;
    let mut max_i = i0;
    for i in i0 + 1..i1 {
        // 点到线段 ab 的垂距
        let ap = pts[i].sub(a);
        let cross = (ab.x * ap.y - ab.y * ap.x).abs();
        let d = cross / ab_len;
        if d > max_d {
            max_d = d;
            max_i = i;
        }
    }
    if max_d > eps {
        keep[max_i] = true;
        rdp_rec(pts, i0, max_i, eps, keep);
        rdp_rec(pts, max_i, i1, eps, keep);
    }
}

/// 便捷：图片 → 简化轮廓多边形（顶点上限约束在 [24, 160]）。
pub fn extract(img: &RgbaImage) -> Vec<V2> {
    let (mask, w, h) = solid_mask(img, ALPHA_SOLID);
    let ring = trace_boundary(&mask, w, h);
    if ring.len() < 3 {
        return ring;
    }
    // 自适应 epsilon：先 1.5px，若点数仍 >160 再加大
    let mut eps = 1.5;
    let mut poly = simplify_rdp(&ring, eps);
    while poly.len() > 160 && eps < 12.0 {
        eps *= 1.6;
        poly = simplify_rdp(&ring, eps);
    }
    poly
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn rect_img(w: u32, h: u32, x0: u32, y0: u32, bw: u32, bh: u32) -> RgbaImage {
        let mut img = RgbaImage::new(w, h);
        for y in y0..y0 + bh {
            for x in x0..x0 + bw {
                img.put_pixel(x, y, Rgba([200, 100, 50, 255]));
            }
        }
        img
    }

    #[test]
    fn mask_keeps_largest_component() {
        let mut img = rect_img(60, 60, 10, 10, 30, 30);
        img.put_pixel(1, 1, Rgba([0, 0, 0, 255])); // 噪点小连通域
        let (mask, w, _h) = solid_mask(&img, ALPHA_SOLID);
        assert!(mask[15 * w + 20]); // 主体内
        assert!(!mask[w + 1]); // 噪点被丢弃
    }

    #[test]
    fn boundary_traces_rectangle() {
        let img = rect_img(60, 60, 10, 10, 30, 20);
        let (mask, w, h) = solid_mask(&img, ALPHA_SOLID);
        let ring = trace_boundary(&mask, w, h);
        assert!(ring.len() >= 4);
        // 边界点全部落在矩形边界上
        for p in &ring {
            let on_x = p.x as u32 == 10 || p.x as u32 == 39;
            let on_y = p.y as u32 == 10 || p.y as u32 == 29;
            assert!(on_x || on_y, "point {:?} not on border", p);
        }
    }

    #[test]
    fn rdp_reduces_rectangle_to_corners() {
        let img = rect_img(80, 80, 10, 10, 50, 40);
        let poly = extract(&img);
        // 矩形简化后应接近 4 角（允许起点重复/首尾）
        assert!(
            poly.len() <= 8,
            "rectangle simplified to {} pts",
            poly.len()
        );
        assert!(poly.len() >= 4);
    }

    #[test]
    fn rdp_keeps_curve_detail() {
        // 圆形边界不应被简化成三角形
        let mut img = RgbaImage::new(80, 80);
        for y in 0..80 {
            for x in 0..80 {
                let dx = x as f64 - 40.0;
                let dy = y as f64 - 40.0;
                if dx * dx + dy * dy <= 30.0 * 30.0 {
                    img.put_pixel(x, y, Rgba([200, 100, 50, 255]));
                }
            }
        }
        let poly = extract(&img);
        assert!(poly.len() >= 10, "circle kept only {} pts", poly.len());
        assert!(poly.len() <= 160);
    }
}
