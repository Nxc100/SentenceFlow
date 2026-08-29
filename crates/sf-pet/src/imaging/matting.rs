//! 本地判别式抠图（环节七）。
//!
//! 模型：u2netp（Apache-2.0，约 4.4MB，随包内嵌）；推理：`tract`（纯 Rust，
//! 零原生依赖、零云端、零 API Key——「零生成式 AI」）。
//! 任何加载/推理失败都返回 Err，调用方（wizard `Busy` 分支）回退泛洪抠图，
//! 保证质量只升不降。
//!
//! **`matting` feature**（默认开）：关掉即整体退化为「模型不可用」，
//! 泛洪抠图仍在，只是复杂背景素材的质量回到 L0 水平。关掉能省下 tract 的
//! 依赖树与 4.4MB 模型 —— CI 快检与不需要抠图的构建可以走这条。

#[cfg(feature = "matting")]
use image::imageops;
use image::{Rgba, RgbaImage};

use crate::error::{PetError, PetResult};

#[cfg(feature = "matting")]
mod engine {
    use std::sync::OnceLock;
    use tract_onnx::prelude::*;

    pub(super) type Model =
        SimplePlan<TypedFact, Box<dyn TypedOp>, Graph<TypedFact, Box<dyn TypedOp>>>;

    const MODEL_BYTES: &[u8] = include_bytes!("../../models/u2netp.onnx");
    pub(super) const SIDE: usize = 320;

    static MODEL: OnceLock<Option<Model>> = OnceLock::new();

    fn build_model() -> TractResult<Model> {
        tract_onnx::onnx()
            .model_for_read(&mut std::io::Cursor::new(MODEL_BYTES))?
            .with_input_fact(0, f32::fact([1, 3, SIDE, SIDE]).into())?
            .into_optimized()?
            .into_runnable()
    }

    /// 懒加载单例：建图约数百毫秒，且进程内可复用。
    pub(super) fn model() -> Option<&'static Model> {
        MODEL
            .get_or_init(|| match build_model() {
                Ok(m) => Some(m),
                Err(e) => {
                    tracing::warn!("u2netp 加载失败，回退泛洪抠图: {e:?}");
                    None
                }
            })
            .as_ref()
    }
}

#[cfg(feature = "matting")]
use engine::SIDE;

#[cfg(feature = "matting")]
pub fn is_available() -> bool {
    engine::model().is_some()
}

/// 未编译语义抠图时恒为「不可用」：调用方据此回退泛洪抠图。
#[cfg(not(feature = "matting"))]
pub fn is_available() -> bool {
    false
}

#[cfg(not(feature = "matting"))]
pub fn segment_alpha(_img: &RgbaImage) -> PetResult<Vec<u8>> {
    Err(PetError::msg("本次构建未包含智能抠图模型"))
}

/// 推理出前景概率蒙版并缩放回原尺寸（返回与像素序一致的 alpha 数组）。
#[cfg(feature = "matting")]
pub fn segment_alpha(img: &RgbaImage) -> PetResult<Vec<u8>> {
    use tract_onnx::prelude::*;
    let model = engine::model().ok_or_else(|| PetError::msg("抠图模型不可用"))?;
    let resized = imageops::resize(
        img,
        SIDE as u32,
        SIDE as u32,
        imageops::FilterType::Triangle,
    );

    // rembg 系 u2net 标准预处理：/255 → 减均值除标准差
    const MEAN: [f32; 3] = [0.485, 0.456, 0.406];
    const STD: [f32; 3] = [0.229, 0.224, 0.225];
    let input = tract_ndarray::Array4::from_shape_fn((1, 3, SIDE, SIDE), |(_, c, y, x)| {
        let px = resized.get_pixel(x as u32, y as u32).0;
        (px[c] as f32 / 255.0 - MEAN[c]) / STD[c]
    });

    let result = model
        .run(tvec!(Tensor::from(input).into()))
        .map_err(|e| PetError::msg(format!("抠图推理失败: {e}")))?;
    // 输出 d0: [1,1,320,320]（模型内已含 Sigmoid）
    let out = result[0]
        .as_slice::<f32>()
        .map_err(|e| PetError::msg(format!("抠图输出异常: {e}")))?;
    if out.len() < SIDE * SIDE {
        return Err(PetError::msg("抠图输出尺寸异常"));
    }

    // min-max 归一化（rembg 同款后处理）
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for v in &out[..SIDE * SIDE] {
        lo = lo.min(*v);
        hi = hi.max(*v);
    }
    let range = (hi - lo).max(1e-6);

    let mut mask = image::GrayImage::new(SIDE as u32, SIDE as u32);
    for y in 0..SIDE {
        for x in 0..SIDE {
            let v = (out[y * SIDE + x] - lo) / range;
            mask.put_pixel(x as u32, y as u32, image::Luma([(v * 255.0) as u8]));
        }
    }
    let full = imageops::resize(
        &mask,
        img.width(),
        img.height(),
        imageops::FilterType::Triangle,
    );
    Ok(full.into_raw())
}

/// 语义抠图：软阈值蒙版应用为 alpha，透明像素 RGB 清零。
pub fn cut_out(img: &RgbaImage) -> PetResult<RgbaImage> {
    let alpha = segment_alpha(img)?;
    let mut out = img.clone();
    for (i, px) in out.pixels_mut().enumerate() {
        let a = alpha.get(i).copied().unwrap_or(0);
        // 软阈值：<30 全透明，>200 全保留，中间线性过渡（保住毛发边）
        let na = if a < 30 {
            0
        } else if a > 200 {
            255
        } else {
            ((a as f32 - 30.0) / 170.0 * 255.0) as u8
        };
        let merged = ((px.0[3] as u16 * na as u16) / 255) as u8;
        if merged < 8 {
            *px = Rgba([0, 0, 0, 0]);
        } else {
            px.0[3] = merged;
        }
    }
    Ok(out)
}

/// 不透明像素占比（用于判断抠图结果是否合理）。
pub fn foreground_fraction(img: &RgbaImage) -> f32 {
    let total = (img.width() * img.height()).max(1) as f32;
    let solid = img
        .pixels()
        .filter(|p| p.0[3] >= super::ALPHA_SOLID)
        .count() as f32;
    solid / total
}

/// 并行批量抠图。抠图是整个管线里唯一的计算热点：tract 是纯 Rust 单线程实现，
/// 实测 u2netp 单帧约 2 秒（onnxruntime 多线程约 0.07 秒，差距全在并行度上）。
/// 视频路线一次要处理多帧，按核心数分片后 24 核机器上基本等同于单帧耗时。
/// `progress` 会在每帧完成时被调用（可能来自任意线程），用于向前端播报进度。
pub fn cut_out_batch(
    frames: &[RgbaImage],
    progress: &(dyn Fn(usize, usize) + Sync),
) -> PetResult<Vec<RgbaImage>> {
    let total = frames.len();
    if total == 0 {
        return Ok(Vec::new());
    }
    // 先在当前线程把模型初始化好，避免多个线程同时进 OnceLock 各建一份图
    if !is_available() {
        return Err(PetError::msg("抠图模型不可用"));
    }

    let workers = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(total)
        .max(1);
    let done = std::sync::atomic::AtomicUsize::new(0);
    // 各帧耗时基本相同，按连续区间均分即可
    let chunk = total.div_ceil(workers);

    let mut results: Vec<PetResult<RgbaImage>> = Vec::with_capacity(total);
    std::thread::scope(|scope| {
        let handles: Vec<_> = frames
            .chunks(chunk)
            .map(|part| {
                let done = &done;
                scope.spawn(move || {
                    part.iter()
                        .map(|f| {
                            let r = cut_out(f);
                            let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                            progress(n, total);
                            r
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        for h in handles {
            match h.join() {
                Ok(part) => results.extend(part),
                Err(_) => results.push(Err(PetError::msg("抠图线程异常退出"))),
            }
        }
    });

    results.into_iter().collect()
}

/// 一组已抠像帧的主体总包围盒，向外留出 `pad_ratio` 的余量。
/// 视频路线用它把「运动分析」限制在主体附近，既准又便宜。
pub fn union_roi(frames: &[RgbaImage], pad_ratio: f32) -> Option<super::layout::Bbox> {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
    let mut found = false;
    for f in frames {
        if let Some(b) = super::layout::bbox(f) {
            found = true;
            x0 = x0.min(b.x0);
            y0 = y0.min(b.y0);
            x1 = x1.max(b.x0 + b.w);
            y1 = y1.max(b.y0 + b.h);
        }
    }
    if !found {
        return None;
    }
    let (fw, fh) = frames[0].dimensions();
    let pad_x = (((x1 - x0) as f32) * pad_ratio) as u32;
    let pad_y = (((y1 - y0) as f32) * pad_ratio) as u32;
    let nx0 = x0.saturating_sub(pad_x);
    let ny0 = y0.saturating_sub(pad_y);
    let nx1 = (x1 + pad_x).min(fw);
    let ny1 = (y1 + pad_y).min(fh);
    Some(super::layout::Bbox {
        x0: nx0,
        y0: ny0,
        w: (nx1 - nx0).max(1),
        h: (ny1 - ny0).max(1),
        centroid_x: (nx0 + nx1) as f32 / 2.0,
    })
}

/// 合理前景占比区间（太小=没抠到，太大=背景没去掉）。
pub fn fraction_plausible(f: f32) -> bool {
    (0.03..=0.90).contains(&f)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(feature = "matting")]
    fn model_loads_and_outputs_full_size_mask() {
        assert!(is_available(), "内嵌 u2netp 应能加载");
        // 类照片输入：渐变背景 + 中央深色主体
        let mut img = RgbaImage::new(160, 200);
        for y in 0..200u32 {
            for x in 0..160u32 {
                let g = 150 + (y / 4) as u8;
                img.put_pixel(x, y, Rgba([g, g, g.saturating_add(10), 255]));
            }
        }
        for y in 50..150 {
            for x in 40..120 {
                img.put_pixel(x, y, Rgba([70, 40, 30, 255]));
            }
        }
        let alpha = segment_alpha(&img).unwrap();
        assert_eq!(alpha.len(), 160 * 200, "蒙版应与原图同尺寸");
        let cut = cut_out(&img).unwrap();
        assert_eq!(cut.dimensions(), (160, 200));
        let f = foreground_fraction(&cut);
        assert!(f > 0.0 && f < 1.0, "前景占比应在开区间内，实际 {f}");
    }

    #[test]
    fn fraction_plausibility_bounds() {
        assert!(!fraction_plausible(0.001));
        assert!(fraction_plausible(0.3));
        assert!(!fraction_plausible(0.97));
    }
}
