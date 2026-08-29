//! 散点降噪：移除小于阈值的孤立不透明连通域（抠色后的绿幕噪点）。

use image::{Rgba, RgbaImage};

use super::ALPHA_SOLID;

/// 移除面积小于 `min_area` 像素的连通域（4 邻接）。
pub fn remove_specks(img: &mut RgbaImage, min_area: u32) {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return;
    }
    let idx = |x: u32, y: u32| (y * w + x) as usize;
    let mut visited = vec![false; (w * h) as usize];

    for sy in 0..h {
        for sx in 0..w {
            if visited[idx(sx, sy)] || img.get_pixel(sx, sy).0[3] < ALPHA_SOLID {
                continue;
            }
            // BFS 收集当前连通域
            let mut component = Vec::new();
            let mut queue = std::collections::VecDeque::new();
            visited[idx(sx, sy)] = true;
            queue.push_back((sx, sy));
            while let Some((x, y)) = queue.pop_front() {
                component.push((x, y));
                let neighbors = [
                    (x.wrapping_sub(1), y),
                    (x + 1, y),
                    (x, y.wrapping_sub(1)),
                    (x, y + 1),
                ];
                for (nx, ny) in neighbors {
                    if nx < w
                        && ny < h
                        && !visited[idx(nx, ny)]
                        && img.get_pixel(nx, ny).0[3] >= ALPHA_SOLID
                    {
                        visited[idx(nx, ny)] = true;
                        queue.push_back((nx, ny));
                    }
                }
            }
            if (component.len() as u32) < min_area {
                for (x, y) in component {
                    img.put_pixel(x, y, Rgba([0, 0, 0, 0]));
                }
            }
        }
    }
}

/// 依据图片尺寸给出建议的降噪阈值。
pub fn default_min_area(w: u32, h: u32) -> u32 {
    ((w * h) as f32 * 0.0004).clamp(12.0, 400.0) as u32
}

/// 只保留主体：丢弃面积不足最大连通域 `min_share` 的**独立**碎块。
///
/// 语义抠像常把地面阴影、脚下反光判成前景（实测走路帧里有个半透明碎块，
/// 面积仅为主体的 3.3%、平均 alpha 44）。这类碎块按绝对面积算够大、
/// 逃得过 `remove_specks`，但它与主体不相连、只在个别帧出现——
/// 播成动画就是脚边一闪一闪的脏东西。用**相对主体**的比例来判更稳：
/// 主体多大都适用，且真正长在身上的部件不会被误伤（它们与主体连通）。
pub fn keep_main_subject(img: &mut RgbaImage, min_share: f32) {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return;
    }
    let idx = |x: u32, y: u32| (y * w + x) as usize;
    let mut visited = vec![false; (w * h) as usize];
    let mut comps: Vec<Vec<(u32, u32)>> = Vec::new();

    for sy in 0..h {
        for sx in 0..w {
            if visited[idx(sx, sy)] || img.get_pixel(sx, sy).0[3] < ALPHA_SOLID {
                continue;
            }
            let mut comp = Vec::new();
            let mut queue = std::collections::VecDeque::new();
            visited[idx(sx, sy)] = true;
            queue.push_back((sx, sy));
            while let Some((x, y)) = queue.pop_front() {
                comp.push((x, y));
                for (nx, ny) in [
                    (x.wrapping_sub(1), y),
                    (x + 1, y),
                    (x, y.wrapping_sub(1)),
                    (x, y + 1),
                ] {
                    if nx < w
                        && ny < h
                        && !visited[idx(nx, ny)]
                        && img.get_pixel(nx, ny).0[3] >= ALPHA_SOLID
                    {
                        visited[idx(nx, ny)] = true;
                        queue.push_back((nx, ny));
                    }
                }
            }
            comps.push(comp);
        }
    }

    let Some(largest) = comps.iter().map(|c| c.len()).max() else {
        return;
    };
    let cutoff = (largest as f32 * min_share) as usize;
    for comp in comps {
        if comp.len() < cutoff {
            for (x, y) in comp {
                img.put_pixel(x, y, Rgba([0, 0, 0, 0]));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_small_keeps_large() {
        let mut img = RgbaImage::new(64, 64);
        // 大主体 20x20
        for y in 10..30 {
            for x in 10..30 {
                img.put_pixel(x, y, Rgba([255, 0, 0, 255]));
            }
        }
        // 噪点 2x2
        for y in 50..52 {
            for x in 50..52 {
                img.put_pixel(x, y, Rgba([0, 255, 0, 255]));
            }
        }
        remove_specks(&mut img, 30);
        assert_eq!(img.get_pixel(15, 15).0[3], 255);
        assert_eq!(img.get_pixel(50, 50).0[3], 0);
    }
}

#[cfg(test)]
mod main_subject_tests {
    use super::*;

    /// 阴影碎块（面积约主体 3%）应被丢弃，主体完好。
    #[test]
    fn drops_detached_shadow_fragment() {
        let mut img = RgbaImage::new(200, 200);
        for y in 20..170 {
            for x in 40..140 {
                img.put_pixel(x, y, Rgba([230, 200, 160, 255])); // 主体 15000px
            }
        }
        for y in 180..192 {
            for x in 150..190 {
                img.put_pixel(x, y, Rgba([210, 210, 210, 44])); // 半透明阴影 480px ≈ 3.2%
            }
        }
        keep_main_subject(&mut img, 0.15);
        assert_eq!(img.get_pixel(90, 90).0[3], 255, "主体必须完好");
        assert_eq!(img.get_pixel(170, 185).0[3], 0, "分离的阴影碎块应被清除");
    }

    /// 长在身上的部件与主体连通，绝不能被误伤。
    #[test]
    fn keeps_connected_parts() {
        let mut img = RgbaImage::new(200, 200);
        for y in 60..140 {
            for x in 60..140 {
                img.put_pixel(x, y, Rgba([230, 200, 160, 255]));
            }
        }
        for y in 96..104 {
            for x in 140..180 {
                img.put_pixel(x, y, Rgba([230, 200, 160, 255])); // 与主体相连的尾巴
            }
        }
        keep_main_subject(&mut img, 0.15);
        assert_eq!(img.get_pixel(170, 100).0[3], 255, "相连的尾巴不应被清除");
    }

    /// 体量相当的第二个部件（如分离绘制的大耳朵）不该被丢。
    #[test]
    fn keeps_large_detached_part() {
        let mut img = RgbaImage::new(200, 200);
        for y in 20..120 {
            for x in 20..120 {
                img.put_pixel(x, y, Rgba([230, 200, 160, 255])); // 10000px
            }
        }
        for y in 140..190 {
            for x in 140..190 {
                img.put_pixel(x, y, Rgba([230, 200, 160, 255])); // 2500px = 25% > 15%
            }
        }
        keep_main_subject(&mut img, 0.15);
        assert_eq!(
            img.get_pixel(160, 160).0[3],
            255,
            "体量足够的分离部件应保留"
        );
    }
}
