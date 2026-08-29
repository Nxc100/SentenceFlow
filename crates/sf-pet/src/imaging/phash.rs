//! 感知哈希（pHash）：32×32 灰度 → 2D DCT → 8×8 低频 → 64bit，
//! 用于帧间"身份漂移"粗检（方案 §8.4 软性校验）。

use image::RgbaImage;

const N: usize = 32;
const LOW: usize = 8;

/// 计算一帧的 64bit pHash。透明像素按中灰合成，降低背景影响。
// DCT 是数学式直译：下标 `u`/`v`/`x`/`y` 就是公式里的自变量，
// 换成迭代器会把 `cos((2y+1)uπ/2N)` 这类项拆散，读起来反而对不上公式。
#[allow(clippy::needless_range_loop)]
pub fn phash(img: &RgbaImage) -> u64 {
    // 1) 缩放到 32×32 灰度（简单盒均值，透明合成中灰 128）
    let (w, h) = img.dimensions();
    let mut gray = [[0f64; N]; N];
    if w == 0 || h == 0 {
        return 0;
    }
    for gy in 0..N {
        for gx in 0..N {
            let x0 = (gx as u32 * w) / N as u32;
            let x1 = (((gx + 1) as u32 * w) / N as u32).max(x0 + 1).min(w);
            let y0 = (gy as u32 * h) / N as u32;
            let y1 = (((gy + 1) as u32 * h) / N as u32).max(y0 + 1).min(h);
            let mut acc = 0f64;
            let mut cnt = 0f64;
            for y in y0..y1 {
                for x in x0..x1 {
                    let [r, g, b, a] = img.get_pixel(x, y).0;
                    let af = a as f64 / 255.0;
                    let lum = 0.299 * r as f64 + 0.587 * g as f64 + 0.114 * b as f64;
                    acc += lum * af + 128.0 * (1.0 - af);
                    cnt += 1.0;
                }
            }
            gray[gy][gx] = acc / cnt;
        }
    }

    // 2) 2D DCT-II（仅需低频 LOW×LOW 输出）
    let mut dct = [[0f64; LOW]; LOW];
    for u in 0..LOW {
        for v in 0..LOW {
            let mut sum = 0f64;
            for y in 0..N {
                for x in 0..N {
                    sum += gray[y][x]
                        * ((std::f64::consts::PI * (2.0 * y as f64 + 1.0) * u as f64)
                            / (2.0 * N as f64))
                            .cos()
                        * ((std::f64::consts::PI * (2.0 * x as f64 + 1.0) * v as f64)
                            / (2.0 * N as f64))
                            .cos();
                }
            }
            dct[u][v] = sum;
        }
    }

    // 3) 除 DC 外取中位数阈值化
    let mut vals: Vec<f64> = Vec::with_capacity(LOW * LOW - 1);
    for u in 0..LOW {
        for v in 0..LOW {
            if u == 0 && v == 0 {
                continue;
            }
            vals.push(dct[u][v]);
        }
    }
    let mut sorted = vals.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = sorted[sorted.len() / 2];

    let mut hash = 0u64;
    let mut bit = 0;
    for u in 0..LOW {
        for v in 0..LOW {
            if u == 0 && v == 0 {
                continue;
            }
            if dct[u][v] > median {
                hash |= 1u64 << bit;
            }
            bit += 1;
        }
    }
    hash
}

pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// 同一状态相邻帧的漂移警戒阈值（动画帧间应远低于此）。
pub const DRIFT_THRESHOLD: u32 = 24;

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn blob(seed: u8) -> RgbaImage {
        let mut img = RgbaImage::new(64, 64);
        for y in 10..54 {
            for x in 14..50 {
                let v = ((x * 3 + y * 5) as u8).wrapping_add(seed);
                img.put_pixel(x as u32, y as u32, Rgba([v, 100, 150, 255]));
            }
        }
        img
    }

    #[test]
    fn identical_frames_zero_distance() {
        assert_eq!(hamming(phash(&blob(0)), phash(&blob(0))), 0);
    }

    #[test]
    fn small_change_small_distance() {
        let a = phash(&blob(0));
        let b = phash(&blob(6));
        assert!(hamming(a, b) < DRIFT_THRESHOLD, "微小变化不应触发漂移告警");
    }

    #[test]
    fn different_subject_large_distance() {
        let a = phash(&blob(0));
        // 完全不同的图案：棋盘
        let mut img = RgbaImage::new(64, 64);
        for y in 0..64 {
            for x in 0..64 {
                if (x / 8 + y / 8) % 2 == 0 {
                    img.put_pixel(x, y, Rgba([255, 255, 255, 255]));
                }
            }
        }
        let b = phash(&img);
        assert!(hamming(a, b) >= DRIFT_THRESHOLD, "不同主体距离应超过阈值");
    }
}
