//! 图像处理管线（方案 §8.1）：色键→alpha、连通域降噪、切帧、质心对齐、pHash。

pub mod chroma;
pub mod decoder;
pub mod denoise;
pub mod layout;
pub mod matting;
pub mod motion;
pub mod phash;
pub mod slice;

use image::RgbaImage;

use crate::error::PetResult;

pub const ALPHA_SOLID: u8 = 8; // alpha >= 此值视为"有内容"像素

/// 无损 WebP 编码（sheet 落盘格式）。
pub fn encode_webp(img: &RgbaImage) -> PetResult<Vec<u8>> {
    let mut buf = Vec::new();
    let enc = image::codecs::webp::WebPEncoder::new_lossless(&mut buf);
    enc.encode(
        img.as_raw(),
        img.width(),
        img.height(),
        image::ExtendedColorType::Rgba8,
    )?;
    Ok(buf)
}

/// PNG 编码（预览缩略图 / 引导图 / 单帧导出）。
pub fn encode_png(img: &RgbaImage) -> PetResult<Vec<u8>> {
    let mut buf = Vec::new();
    let enc = image::codecs::png::PngEncoder::new(&mut buf);
    image::ImageEncoder::write_image(
        enc,
        img.as_raw(),
        img.width(),
        img.height(),
        image::ExtendedColorType::Rgba8,
    )?;
    Ok(buf)
}

pub fn to_png_data_url(img: &RgbaImage) -> PetResult<String> {
    use base64::Engine as _;
    let png = encode_png(img)?;
    Ok(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(png)
    ))
}

/// RGB → HSV，h ∈ [0,360)，s、v ∈ [0,1]。
pub fn rgb_to_hsv(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let s = if max == 0.0 { 0.0 } else { d / max };
    (h, s, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsv_pure_green() {
        let (h, s, v) = rgb_to_hsv(0, 255, 0);
        assert!((h - 120.0).abs() < 0.5);
        assert!(s > 0.99 && v > 0.99);
    }

    #[test]
    fn webp_round_trip() {
        let mut img = RgbaImage::new(16, 16);
        img.put_pixel(3, 4, image::Rgba([255, 0, 0, 255]));
        let bytes = encode_webp(&img).unwrap();
        let back = image::load_from_memory(&bytes).unwrap().to_rgba8();
        assert_eq!(back.get_pixel(3, 4).0, [255, 0, 0, 255]);
        assert_eq!(back.get_pixel(0, 0).0[3], 0);
    }
}
