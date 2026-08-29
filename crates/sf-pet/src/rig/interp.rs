#![allow(dead_code)]
// P2 可选路径：rigged 宠物由 bake 加密相位采样即得丝滑；本模块服务离散
// 关键姿势通路，等待导入 UI 触发，作为已测试公开能力保留（RIFE 为其升级）。

//! 插帧（§3.6 P2 / §16.3）：把少量关键姿势补成顺滑循环。
//!
//! 重要工程观察：本引擎的**骨骼 rig 是连续的**——bake 可在任意相位直接采样，故「丝滑档」
//! 由**加密相位采样**天然达成（bake 里把 frames 调大即可），插帧对 rigged 宠物并非必需。
//! 本模块服务的是**离散关键姿势**通路（输入=手绘/历史成功生成物/视频抽帧，无 rig）：
//! 提供确定性 CPU 交叉混合兜底；RIFE 轻量 ONNX（MIT，复用 tract-onnx 与按需下载器）为
//! P2 可选质量升级（光流形变优于交叉混合），闸门=循环可用≥70%、<5s/帧、包体≤15MB。

use image::{Rgba, RgbaImage};

/// 在关键帧间线性交叉混合，生成 out_count 帧的循环序列（首帧对齐 keys[0]）。
/// keys 至少 2 张、尺寸一致。out_count < keys.len() 时退化为原样截取。
pub fn crossfade_loop(keys: &[RgbaImage], out_count: usize) -> Vec<RgbaImage> {
    if keys.len() < 2 || out_count == 0 {
        return keys.to_vec();
    }
    let (w, h) = keys[0].dimensions();
    let seg = keys.len(); // 循环：keys[n-1] → keys[0]
    (0..out_count)
        .map(|i| {
            let x = i as f64 / out_count as f64 * seg as f64;
            let a = x.floor() as usize % keys.len();
            let b = (a + 1) % keys.len();
            let t = x - x.floor();
            blend(&keys[a], &keys[b], t, w, h)
        })
        .collect()
}

fn blend(a: &RgbaImage, b: &RgbaImage, t: f64, w: u32, h: u32) -> RgbaImage {
    let mut out = RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let pa = a.get_pixel(x, y).0;
            let pb = b.get_pixel(x, y).0;
            let mut px = [0u8; 4];
            for c in 0..4 {
                px[c] = (pa[c] as f64 * (1.0 - t) + pb[c] as f64 * t).round() as u8;
            }
            out.put_pixel(x, y, Rgba(px));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: u32, h: u32, c: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(w, h, Rgba(c))
    }

    #[test]
    fn produces_requested_count() {
        let keys = vec![solid(8, 8, [255, 0, 0, 255]), solid(8, 8, [0, 0, 255, 255])];
        let out = crossfade_loop(&keys, 8);
        assert_eq!(out.len(), 8);
    }

    #[test]
    fn midpoint_blends_two_keys() {
        let keys = vec![solid(4, 4, [200, 0, 0, 255]), solid(4, 4, [0, 200, 0, 255])];
        // out_count=4, seg=2 → i=1 → x=0.5 → a=0,b=1,t=0.5：R/G 各半
        let out = crossfade_loop(&keys, 4);
        let p = out[1].get_pixel(0, 0).0;
        assert!((p[0] as i32 - 100).abs() <= 2 && (p[1] as i32 - 100).abs() <= 2);
    }
}
