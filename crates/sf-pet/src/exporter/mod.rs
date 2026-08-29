//! 出生视频（方案 §6 P0-6）：15 秒竖版 mp4。
//! 故事板：孵蛋摇晃 → 裂纹 → 白闪破壳 → 宠物登场撒花 → 各状态走秀 → 名字落版（+水印）。
//! 帧渲染与 ffmpeg 编码解耦：`render_birth_video` 只产帧，便于单测。

pub mod ffmpeg;

use std::collections::HashMap;

use image::{Rgba, RgbaImage, imageops};

use crate::error::PetResult;
use crate::imaging::layout;
use crate::petkit::{PetSpec, StateId};

pub const VID_W: u32 = 720;
pub const VID_H: u32 = 1280;
pub const FPS: u32 = 30;
pub const DURATION_SECS: f32 = 15.0;

// ---------------------------------------------------------------- 小工具

/// 确定性伪随机（粒子布置用，避免每次导出不同引发测试不稳定）。
struct Rng(u64);

impl Rng {
    fn f32(&mut self) -> f32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 40) as f32) / (1u64 << 24) as f32
    }
    fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.f32()
    }
}

fn fill_rect(img: &mut RgbaImage, x: i64, y: i64, w: u32, h: u32, color: Rgba<u8>) {
    let (iw, ih) = img.dimensions();
    let alpha = color.0[3] as u32;
    for dy in 0..h as i64 {
        for dx in 0..w as i64 {
            let (px, py) = (x + dx, y + dy);
            if px >= 0 && py >= 0 && (px as u32) < iw && (py as u32) < ih {
                if alpha == 255 {
                    img.put_pixel(px as u32, py as u32, color);
                } else {
                    let dst = img.get_pixel_mut(px as u32, py as u32);
                    for c in 0..3 {
                        dst.0[c] = ((dst.0[c] as u32 * (255 - alpha) + color.0[c] as u32 * alpha)
                            / 255) as u8;
                    }
                }
            }
        }
    }
}

fn white_flash(img: &mut RgbaImage, alpha: f32) {
    let a = (alpha.clamp(0.0, 1.0) * 255.0) as u32;
    if a == 0 {
        return;
    }
    for px in img.pixels_mut() {
        for c in 0..3 {
            px.0[c] = ((px.0[c] as u32 * (255 - a) + 255 * a) / 255) as u8;
        }
    }
}

/// 粗线段（DDA + 方形笔刷）。
fn draw_line(img: &mut RgbaImage, x0: f32, y0: f32, x1: f32, y1: f32, t: u32, color: Rgba<u8>) {
    let steps = ((x1 - x0).abs().max((y1 - y0).abs()) as u32).max(1);
    for i in 0..=steps {
        let k = i as f32 / steps as f32;
        let x = x0 + (x1 - x0) * k;
        let y = y0 + (y1 - y0) * k;
        fill_rect(
            img,
            x as i64 - t as i64 / 2,
            y as i64 - t as i64 / 2,
            t,
            t,
            color,
        );
    }
}

// ---------------------------------------------------------------- 场景元素

/// 渐变背景（预渲染一次）。
fn make_background() -> RgbaImage {
    let top = [255u8, 236, 214];
    let bottom = [226u8, 216, 255];
    let mut bg = RgbaImage::new(VID_W, VID_H);
    for y in 0..VID_H {
        let k = y as f32 / VID_H as f32;
        let px = Rgba([
            (top[0] as f32 + (bottom[0] as f32 - top[0] as f32) * k) as u8,
            (top[1] as f32 + (bottom[1] as f32 - top[1] as f32) * k) as u8,
            (top[2] as f32 + (bottom[2] as f32 - top[2] as f32) * k) as u8,
            255,
        ]);
        for x in 0..VID_W {
            bg.put_pixel(x, y, px);
        }
    }
    bg
}

const EGG_W: u32 = 320;
const EGG_H: u32 = 400;

/// 程序化蛋：蛋形轮廓 + 奶油填充 + 深色描边 + 腮红。
fn make_egg() -> RgbaImage {
    let mut egg = RgbaImage::new(EGG_W, EGG_H);
    let cx = EGG_W as f32 / 2.0;
    let a = EGG_W as f32 * 0.42;
    let b = EGG_H as f32 * 0.46;
    let cy = EGG_H as f32 * 0.5;
    let shell = Rgba([255, 247, 224, 255]);
    let outline = Rgba([84, 58, 44, 255]);
    for y in 0..EGG_H {
        let ny = (y as f32 - cy) / b; // -1..1
        if ny.abs() > 1.0 {
            continue;
        }
        // 蛋形：底部略宽
        let half = a * (1.0 - ny * ny).sqrt() * (1.0 + 0.16 * ny);
        for x in 0..EGG_W {
            let dx = (x as f32 - cx).abs();
            if dx <= half {
                let color = if half - dx < 7.0 || (1.0 - ny.abs()) * b < 7.0 {
                    outline
                } else {
                    shell
                };
                egg.put_pixel(x, y, color);
            }
        }
    }
    // 腮红
    let blush = Rgba([255, 170, 180, 200]);
    fill_rect(
        &mut egg,
        (cx - a * 0.62) as i64,
        (cy + b * 0.1) as i64,
        34,
        18,
        blush,
    );
    fill_rect(
        &mut egg,
        (cx + a * 0.62 - 34.0) as i64,
        (cy + b * 0.1) as i64,
        34,
        18,
        blush,
    );
    // 眼睛
    let eye = Rgba([60, 42, 32, 255]);
    fill_rect(
        &mut egg,
        (cx - 46.0) as i64,
        (cy - b * 0.15) as i64,
        16,
        22,
        eye,
    );
    fill_rect(
        &mut egg,
        (cx + 30.0) as i64,
        (cy - b * 0.15) as i64,
        16,
        22,
        eye,
    );
    egg
}

/// 裂纹折线（相对蛋图坐标），progress ∈ [0,1] 控制画出的段数。
fn draw_crack(egg: &mut RgbaImage, progress: f32) {
    if progress <= 0.0 {
        return;
    }
    let pts = [
        (28.0, 216.0),
        (74.0, 196.0),
        (118.0, 226.0),
        (162.0, 192.0),
        (206.0, 228.0),
        (250.0, 198.0),
        (292.0, 218.0),
    ];
    let total = pts.len() - 1;
    let visible = ((progress * total as f32).ceil() as usize).min(total);
    let dark = Rgba([84, 58, 44, 255]);
    for i in 0..visible {
        let (x0, y0) = pts[i];
        let (x1, y1) = pts[i + 1];
        draw_line(egg, x0, y0, x1, y1, 6, dark);
    }
}

const CONFETTI_COLORS: [[u8; 3]; 5] = [
    [255, 107, 138],
    [255, 209, 102],
    [107, 203, 119],
    [77, 150, 255],
    [199, 125, 255],
];

/// 撒花：t0 起播，每片确定性参数。
fn draw_confetti(frame: &mut RgbaImage, t: f32, t0: f32, count: u32, seed: u64) {
    if t < t0 {
        return;
    }
    let mut rng = Rng(seed);
    for i in 0..count {
        let x0 = rng.range(0.0, VID_W as f32);
        let speed = rng.range(120.0, 300.0);
        let sway = rng.range(20.0, 60.0);
        let phase = rng.range(0.0, std::f32::consts::TAU);
        let size = rng.range(7.0, 14.0) as u32;
        let delay = rng.range(0.0, 1.6);
        let color = CONFETTI_COLORS[(i % 5) as usize];
        let life = t - t0 - delay;
        if life < 0.0 {
            continue;
        }
        let y = -20.0 + speed * life;
        if y > VID_H as f32 + 20.0 {
            continue;
        }
        let x = x0 + (life * 2.4 + phase).sin() * sway;
        fill_rect(
            frame,
            x as i64,
            y as i64,
            size,
            (size as f32 * 0.6) as u32,
            Rgba([color[0], color[1], color[2], 235]),
        );
    }
}

/// 破壳蛋壳碎片。
fn draw_shards(frame: &mut RgbaImage, life: f32, cx: f32, cy: f32) {
    if life < 0.0 {
        return;
    }
    let mut rng = Rng(7);
    for _ in 0..14 {
        let angle = rng.range(0.0, std::f32::consts::TAU);
        let speed = rng.range(260.0, 620.0);
        let size = rng.range(10.0, 22.0) as u32;
        let x = cx + angle.cos() * speed * life;
        let y = cy + angle.sin() * speed * life + 400.0 * life * life;
        fill_rect(
            frame,
            x as i64,
            y as i64,
            size,
            size,
            Rgba([255, 247, 224, 255]),
        );
        fill_rect(
            frame,
            x as i64 + 2,
            y as i64 + 2,
            size - 4,
            size - 4,
            Rgba([255, 240, 200, 255]),
        );
    }
}

// ---------------------------------------------------------------- 宠物帧

const PET_SCALE: u32 = 3;

struct PetFrames {
    /// (row, col) → 放大后的帧
    cache: HashMap<(u32, u32), RgbaImage>,
}

impl PetFrames {
    fn new() -> Self {
        PetFrames {
            cache: HashMap::new(),
        }
    }

    fn get(&mut self, sheet: &RgbaImage, row: u32, col: u32) -> &RgbaImage {
        self.cache.entry((row, col)).or_insert_with(|| {
            let cell = layout::cell_at(sheet, row, col);
            imageops::resize(
                &cell,
                cell.width() * PET_SCALE,
                cell.height() * PET_SCALE,
                imageops::FilterType::Nearest,
            )
        })
    }
}

/// 走秀用状态槽（可用性过滤后不足 5 个用 idle 补齐）。
fn showcase_states(spec: &PetSpec) -> Vec<StateId> {
    let wanted = [
        StateId::Happy,
        StateId::Eat,
        StateId::WalkRight,
        StateId::Sleep,
        StateId::Remind,
    ];
    let mut out: Vec<StateId> = wanted
        .into_iter()
        .filter(|s| {
            spec.clip(*s)
                .map(|c| c.mirror_of.is_none())
                .unwrap_or(false)
        })
        .collect();
    while out.len() < 5 {
        out.push(StateId::Idle);
    }
    out.truncate(5);
    out
}

fn state_frame_index(spec: &PetSpec, state: StateId, local_t: f32) -> (u32, u32) {
    let Some(clip) = spec.clip(state) else {
        return (0, 0);
    };
    let n = clip.frames.max(1);
    let idx = ((local_t * clip.fps as f32) as u32) % n;
    (clip.row, idx)
}

// ---------------------------------------------------------------- 主渲染

pub struct BirthVideoJob<'a> {
    pub spec: &'a PetSpec,
    pub sheet: &'a RgbaImage,
    /// 前端渲染的名字落版（含中文，由 WebView 出图）
    pub name_card: Option<RgbaImage>,
    /// 免费版角标水印
    pub watermark: Option<RgbaImage>,
    /// AI 生成内容显式标识（《标识办法》红线，改进方案 §5.4；全版本叠加，左下角）
    pub ai_notice: Option<RgbaImage>,
}

/// 渲染 15s×30fps 全部帧，逐帧交给 sink（帧序号从 0 起）。
pub fn render_birth_video(
    job: &BirthVideoJob<'_>,
    mut sink: impl FnMut(&RgbaImage, usize, usize) -> PetResult<()>,
) -> PetResult<()> {
    let total = (DURATION_SECS * FPS as f32) as usize;
    let bg = make_background();
    let egg = make_egg();
    let mut pet = PetFrames::new();

    for i in 0..total {
        let t = i as f32 / FPS as f32;
        let frame = render_frame(t, &bg, &egg, job, &mut pet);
        sink(&frame, i, total)?;
    }
    Ok(())
}

/// 单帧渲染（按故事板时间轴），独立成函数便于单测。
fn render_frame(
    t: f32,
    bg: &RgbaImage,
    egg_base: &RgbaImage,
    job: &BirthVideoJob<'_>,
    pet: &mut PetFrames,
) -> RgbaImage {
    let mut frame = bg.clone();
    let (cx, cy) = (VID_W as f32 / 2.0, VID_H as f32 * 0.5);

    if t < 3.4 {
        // —— 孵蛋 ——
        let mut egg = egg_base.clone();
        let crack = ((t - 1.6) / 1.2).clamp(0.0, 1.0);
        draw_crack(&mut egg, crack);
        let amp = (t / 2.5).min(1.0);
        let wobble = (t * 8.0).sin() * 14.0 * amp;
        let ex = cx - EGG_W as f32 / 2.0 + wobble;
        let ey = cy - EGG_H as f32 / 2.0;
        imageops::overlay(&mut frame, &egg, ex as i64, ey as i64);
        if t >= 3.0 {
            draw_shards(&mut frame, t - 3.0, cx, cy);
            white_flash(&mut frame, ((t - 3.0) / 0.15).min(1.0));
        }
    } else {
        // —— 宠物登场 / 走秀 / 落版 ——
        let (row, col);
        let mut pet_x = cx - (192 * PET_SCALE) as f32 / 2.0;
        let mut pet_y = cy - (208 * PET_SCALE) as f32 / 2.0;

        if t < 6.0 {
            (row, col) = state_frame_index(job.spec, StateId::Idle, t - 3.4);
            // 登场弹性缩放感：用轻微上下位移代替缩放（保持像素锐利）
            let k = ((t - 3.4) * 6.0).min(std::f32::consts::PI);
            pet_y -= k.sin() * 40.0;
        } else if t < 13.0 {
            let slot = (((t - 6.0) / 1.4) as usize).min(4);
            let local = (t - 6.0) % 1.4;
            let state = showcase_states(job.spec)[slot];
            (row, col) = state_frame_index(job.spec, state, local);
            if state == StateId::WalkRight {
                pet_x = 40.0 + (VID_W as f32 - 660.0) * (local / 1.4) * 2.0;
            }
        } else {
            (row, col) = state_frame_index(job.spec, StateId::Idle, t);
            pet_y = VID_H as f32 * 0.24 - (208 * PET_SCALE) as f32 / 2.0;
        }

        let sprite = pet.get(job.sheet, row, col);
        imageops::overlay(&mut frame, sprite, pet_x as i64, pet_y as i64);

        // 撒花
        if t < 8.0 {
            draw_confetti(&mut frame, t, 3.4, 70, 42);
        }
        if t >= 13.0 {
            draw_confetti(&mut frame, t, 13.0, 30, 99);
            // 名字落版淡入
            if let Some(card) = &job.name_card {
                let fade = ((t - 13.0) / 0.6).clamp(0.0, 1.0);
                let mut card = card.clone();
                for px in card.pixels_mut() {
                    px.0[3] = (px.0[3] as f32 * fade) as u8;
                }
                let x = (VID_W as i64 - card.width() as i64) / 2;
                let y = (VID_H as f32 * 0.60) as i64;
                imageops::overlay(&mut frame, &card, x, y);
            }
        }
        // 破壳白闪余晖
        if t < 3.9 {
            white_flash(&mut frame, 1.0 - (t - 3.4) / 0.5);
        }
    }

    // —— 水印（免费版，右下角标）——
    if let Some(wm) = &job.watermark {
        let x = VID_W as i64 - wm.width() as i64 - 20;
        let y = VID_H as i64 - wm.height() as i64 - 24;
        imageops::overlay(&mut frame, wm, x, y);
    }
    // —— AI 生成内容显式标识（全版本，左下角，§5.4 红线二）——
    if let Some(notice) = &job.ai_notice {
        let y = VID_H as i64 - notice.height() as i64 - 24;
        imageops::overlay(&mut frame, notice, 20, y);
    }
    frame
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::imaging::layout::{compose_sheet, fit_into_cell};
    use crate::petkit::StateClip;
    use std::collections::BTreeMap;

    fn tiny_pet() -> (PetSpec, RgbaImage) {
        let mut spec = PetSpec::new("测试", "L1");
        spec.states.insert(
            "idle".into(),
            StateClip {
                row: 0,
                frames: 2,
                fps: 8,
                looping: true,
                mirror_of: None,
                source: None,
            },
        );
        let mut blob = RgbaImage::new(100, 120);
        for y in 10..110 {
            for x in 10..90 {
                blob.put_pixel(x, y, Rgba([250, 120, 90, 255]));
            }
        }
        let mut rows = BTreeMap::new();
        rows.insert(
            StateId::Idle,
            vec![fit_into_cell(&blob, 1.0), fit_into_cell(&blob, 1.0)],
        );
        (spec, compose_sheet(&rows))
    }

    #[test]
    fn frames_have_video_dimensions_and_content() {
        let (spec, sheet) = tiny_pet();
        let job = BirthVideoJob {
            spec: &spec,
            sheet: &sheet,
            name_card: None,
            watermark: None,
            ai_notice: None,
        };
        let bg = make_background();
        let egg = make_egg();
        let mut pet = PetFrames::new();
        for t in [0.5f32, 2.0, 3.2, 4.0, 8.0, 14.0] {
            let f = render_frame(t, &bg, &egg, &job, &mut pet);
            assert_eq!(f.dimensions(), (VID_W, VID_H));
            // 每帧都应该比纯背景多点东西（蛋或宠物）。
            // t>=13 落版段宠物移到画面上部（y≈0.24H），取样点跟随故事板。
            let sample_y = if t >= 13.0 {
                (VID_H as f32 * 0.3) as u32
            } else {
                VID_H / 2
            };
            let bg_px = bg.get_pixel(VID_W / 2, sample_y).0;
            let center_differs =
                (0..40).any(|d| f.get_pixel(VID_W / 2 - 20 + d, sample_y).0 != bg_px);
            assert!(center_differs, "t={t} 故事板取样点不应是纯背景");
        }
    }

    #[test]
    fn total_frame_count_is_450() {
        let (spec, sheet) = tiny_pet();
        let job = BirthVideoJob {
            spec: &spec,
            sheet: &sheet,
            name_card: None,
            watermark: None,
            ai_notice: None,
        };
        // 只统计帧数：sink 立即返回
        let mut count = 0usize;
        let mut expected_total = 0usize;
        render_birth_video(&job, |_, i, total| {
            count = i + 1;
            expected_total = total;
            Ok(())
        })
        .unwrap();
        assert_eq!(count, 450);
        assert_eq!(expected_total, 450);
    }

    #[test]
    fn showcase_pads_with_idle() {
        let (spec, _) = tiny_pet();
        let states = showcase_states(&spec);
        assert_eq!(states.len(), 5);
        assert!(states.iter().all(|s| *s == StateId::Idle));
    }
}
