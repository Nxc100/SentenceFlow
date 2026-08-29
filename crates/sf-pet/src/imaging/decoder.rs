//! 视频抽帧路线（改进方案 §8）：ffmpeg 解码 → 帧选择 → 与图片管线同构的帧序列。
//!
//! 解码用 `-f image2pipe -vcodec bmp`：BMP 头部自带文件长度（偏移 2 的 LE u32），
//! 流式分帧是确定性的，不依赖脆弱的字节签名扫描。
//! 帧选择：包围盒稳定性过滤 → 相邻 pHash 去重 → 首尾闭环检测 →（不闭合时）正放+倒放拼接
//! → 等距取帧。

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

use image::RgbaImage;

use crate::error::{PetError, PetResult};
#[cfg(test)]
use crate::imaging::layout;
use crate::imaging::motion;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 抽帧参数：10fps、最长 12s（即梦时长上限）、处理宽度 ≤512。
pub struct ExtractOptions {
    pub fps: u32,
    pub max_secs: f32,
    pub max_width: u32,
}

impl Default for ExtractOptions {
    fn default() -> Self {
        ExtractOptions {
            fps: 10,
            max_secs: 12.0,
            max_width: 512,
        }
    }
}

/// 调 ffmpeg 把视频解为 RGBA 帧序列。
pub fn extract_frames(
    ffmpeg: &Path,
    video: &Path,
    opts: &ExtractOptions,
) -> PetResult<Vec<RgbaImage>> {
    let mut cmd = Command::new(ffmpeg);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let vf = format!("fps={},scale='min({},iw)':-2", opts.fps, opts.max_width);
    let mut child = cmd
        .arg("-i")
        .arg(video)
        .args([
            "-t",
            &format!("{}", opts.max_secs),
            "-vf",
            &vf,
            "-f",
            "image2pipe",
            "-vcodec",
            "bmp",
            "pipe:1",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .spawn()
        .map_err(|e| PetError::msg(format!("无法启动 ffmpeg: {e}")))?;

    let mut raw = Vec::new();
    child
        .stdout
        .take()
        .ok_or_else(|| PetError::msg("ffmpeg 输出管道异常"))?
        .read_to_end(&mut raw)?;
    let status = child.wait()?;
    if !status.success() && raw.is_empty() {
        return Err(PetError::msg(
            "视频解码失败：请确认文件是有效的 mp4/mov 视频",
        ));
    }

    let frames: Vec<RgbaImage> = split_bmp_stream(&raw)
        .into_iter()
        .filter_map(|chunk| image::load_from_memory(chunk).ok().map(|i| i.to_rgba8()))
        .collect();
    if frames.is_empty() {
        return Err(PetError::msg("未能从视频中解出任何帧"));
    }
    Ok(frames)
}

/// 按 BMP 头长度字段切分连续 BMP 流（纯函数，可单测）。
pub fn split_bmp_stream(bytes: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while pos + 6 <= bytes.len() {
        if &bytes[pos..pos + 2] != b"BM" {
            break;
        }
        let size = u32::from_le_bytes([
            bytes[pos + 2],
            bytes[pos + 3],
            bytes[pos + 4],
            bytes[pos + 5],
        ]) as usize;
        if size < 14 || pos + size > bytes.len() {
            break;
        }
        out.push(&bytes[pos..pos + size]);
        pos += size;
    }
    out
}

// ---------------------------------------------------------------- 帧选择器

/// 选择结果：`indices` 为按播放顺序取用的源帧下标（正倒放拼接时会出现重复）。
#[derive(Debug)]
pub struct FrameSelection {
    pub indices: Vec<usize>,
    /// 找到了真闭环（取其中一个完整周期，无需正倒放兜底）
    pub loop_closed: bool,
    /// 闭合不佳，启用了正放+倒放拼接
    pub used_pingpong: bool,
    /// 因包围盒不稳定被丢弃的帧数
    pub dropped_unstable: usize,
    /// 检测到的循环周期（帧数），未找到闭环时为 None
    pub loop_period: Option<usize>,
}

const AREA_DEVIATION_MAX: f32 = 0.35;
/// 去重阈值 = 本片自身逐帧步幅中位数 × 此比例。
/// **相对**阈值是关键：待机的呼吸和走路的迈步幅度差一个数量级，
/// 任何绝对阈值都会在其中一端失效（旧实现的绝对 pHash 阈值就死在这里）。
const DEDUP_RATIO: f32 = 0.5;

/// 步幅超过本段中位数这个倍数即视为「动作切换」，在此切开。
const TRANSITION_SPIKE: f32 = 2.6;
/// 切分后保留的最短连续段（少于此则认为切过头，退回不切）。
const MIN_RUN: usize = 3;

/// 按「转场尖峰」切分，返回最长的一段连续同一动作及其步幅序列。
///
/// 判据是**相对**的（本段步幅中位数的倍数）：走路的步幅本就比呼吸大一个量级，
/// 任何绝对阈值都会在其中一端失效。
fn split_at_transitions(stable: &[usize], steps: &[f32]) -> (Vec<usize>, Vec<f32>) {
    if steps.len() < MIN_RUN {
        return (stable.to_vec(), steps.to_vec());
    }
    let mut sorted = steps.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let median = sorted[sorted.len() / 2];
    if median <= f32::EPSILON {
        return (stable.to_vec(), steps.to_vec());
    }
    let cut = median * TRANSITION_SPIKE;

    // steps[i] 连接 stable[i] 与 stable[i+1]；尖峰处断开
    let mut runs: Vec<(usize, usize)> = Vec::new(); // [start, end) over stable
    let mut start = 0usize;
    for (i, s) in steps.iter().enumerate() {
        if *s > cut {
            if i + 1 - start >= MIN_RUN {
                runs.push((start, i + 1));
            }
            start = i + 1;
        }
    }
    if stable.len() - start >= MIN_RUN {
        runs.push((start, stable.len()));
    }

    match runs.into_iter().max_by_key(|(a, b)| b - a) {
        // 没切出足够长的段（整段都在剧烈变化）→ 保持原样，交给后续环节
        None => (stable.to_vec(), steps.to_vec()),
        Some((a, b)) => (
            stable[a..b].to_vec(),
            steps[a..b.saturating_sub(1)].to_vec(),
        ),
    }
}

/// 从已抠像的帧池中选出 `target` 帧动画。
/// 生产路径统一走 `select_from_analysis`（视频路线要把分析和抠像解耦），
/// 这里保留一个"帧进帧出"的入口给单测用。
#[cfg(test)]
pub fn select_frames(processed: &[RgbaImage], target: usize) -> PetResult<FrameSelection> {
    let areas: Vec<Option<f32>> = processed
        .iter()
        .map(|f| layout::bbox(f).map(|b| (b.w * b.h) as f32))
        .collect();
    let thumbs = motion::thumbs_from_cut(processed);
    select_from_analysis(&thumbs, &areas, target)
}

/// 选帧内核：只依赖「分析缩略图 + 每帧主体面积」，与帧从哪来无关。
///
/// 视频路线用它把 AI 抠像和运动分析解耦——分析跑在原始帧的主体 ROI 上（零推理成本），
/// 只有最终选中的那几帧才真正抠像。`areas[i] = None` 表示该帧面积未知
/// （原始帧没有 alpha），此时跳过稳定性过滤，改由抠像后的空帧检查兜底。
pub fn select_from_analysis(
    thumbs: &[RgbaImage],
    areas: &[Option<f32>],
    target: usize,
) -> PetResult<FrameSelection> {
    let target = target.clamp(2, 8);
    if thumbs.len() < 2 {
        return Err(PetError::msg("视频里可用的帧不足 2 帧，请换一段更长的视频"));
    }

    // 1) 包围盒稳定性过滤：面积偏离中位数 >35% 视为镜头推拉/主体时隐时现
    let known: Vec<f32> = areas.iter().flatten().copied().collect();
    let stable: Vec<usize> = if known.len() < 2 {
        (0..thumbs.len()).collect()
    } else {
        let mut sorted = known.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = sorted[sorted.len() / 2].max(1.0);
        (0..thumbs.len())
            .filter(|&i| {
                areas
                    .get(i)
                    .and_then(|a| *a)
                    .map(|a| ((a - median) / median).abs() <= AREA_DEVIATION_MAX)
                    .unwrap_or(true)
            })
            .collect()
    };
    let dropped_unstable = thumbs.len() - stable.len();
    if stable.len() < 2 {
        return Err(PetError::msg(
            "视频里能稳定看到角色的帧不足 2 帧：镜头有推拉或主体时隐时现，换一段固定镜头的视频再试",
        ));
    }

    let st: Vec<RgbaImage> = stable.iter().map(|&i| thumbs[i].clone()).collect();

    // 2) 整段究竟动没动（绝对阈值，见 motion 模块的标定说明）
    if motion::peak_motion(&st) < motion::STATIC_FLOOR {
        return Err(PetError::msg(
            "这段视频里角色几乎没有动（整段接近一张静止图）：换一段有明显动作的视频，或改用网格图路线",
        ));
    }

    let all_steps: Vec<f32> = (0..st.len() - 1)
        .map(|i| motion::motion_between(&st[i], &st[i + 1]))
        .collect();

    // 3) 转场切分：一个片段里可能混进相邻动作的头尾（脚本时间轴总有误差，
    //    比如"开心"窗口尾部已经开始趴下）。步幅远超本段中位数的地方就是动作切换点，
    //    在那里切开、只保留最长的一段连续同一动作 —— 否则抽出来的帧会是
    //    "侧走 + 正面站 + 趴下"的大杂烩，看着就是一帧一个样。
    let (stable, _steps) = split_at_transitions(&stable, &all_steps);
    let st: Vec<RgbaImage> = stable.iter().map(|&i| thumbs[i].clone()).collect();

    // 4) 循环窗口检测：找到真周期就只取一个周期，动画才是"一次完整的呼吸/步态"，
    //    而不是横跨整段素材的慢速渐变。
    let min_period = target.max(motion::MIN_LOOP_FRAMES);
    let window = motion::best_loop(&st, min_period);

    let (sequence, loop_closed, used_pingpong, loop_period) = match window {
        Some(w) if w.closed() => (
            (w.start..w.start + w.period)
                .map(|i| stable[i])
                .collect::<Vec<_>>(),
            true,
            false,
            Some(w.period),
        ),
        other => {
            // 不闭环。关键：**不要拿整段去正倒放**。
            //
            // 脚本时间轴总有误差，一个片段窗口里很容易混进相邻动作的头尾
            // （实测「开心」窗口里同时有侧走、正面走来、趴下三种姿态）。
            // 拿整段采样出来就是一帧一个样。这里退而取 `best_loop` 找到的
            // **最自洽窗口**——即使首尾没对齐到可判闭环，它仍是这段素材里
            // 最像"同一个动作"的一截——只在它内部去重 + 正倒放。
            let range: Vec<usize> = match other {
                Some(w) => stable[w.start..(w.start + w.period + 1).min(stable.len())].to_vec(),
                None => stable.clone(),
            };
            let sub: Vec<f32> = (0..range.len().saturating_sub(1))
                .map(|i| motion::motion_between(&thumbs[range[i]], &thumbs[range[i + 1]]))
                .collect();

            let mut sorted = sub.clone();
            sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
            let median_step = sorted.get(sorted.len() / 2).copied().unwrap_or(0.0);
            let threshold = median_step * DEDUP_RATIO;

            let mut kept = vec![range[0]];
            let mut acc = 0f32;
            for k in 1..range.len() {
                acc += sub[k - 1];
                if acc >= threshold {
                    kept.push(range[k]);
                    acc = 0.0;
                }
            }
            if kept.len() < 2 {
                kept = range;
            }
            let mut seq = kept.clone();
            seq.extend(kept.iter().rev().skip(1).take(kept.len().saturating_sub(2)));
            (seq, false, true, None)
        }
    };

    // 4) 等距取 target 帧
    let n = sequence.len();
    let take = target.min(n);
    let indices: Vec<usize> = (0..take).map(|k| sequence[k * n / take]).collect();

    Ok(FrameSelection {
        indices,
        loop_closed,
        used_pingpong,
        dropped_unstable,
        loop_period,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::imaging::phash;
    use image::Rgba;

    fn blob_frame(cx: u32, size: u32) -> RgbaImage {
        let mut img = RgbaImage::new(200, 120);
        for y in 30..30 + size {
            for x in cx..cx + size {
                if x < 200 && y < 120 {
                    img.put_pixel(x, y, Rgba([200, 90, 60, 255]));
                }
            }
        }
        img
    }

    #[test]
    fn bmp_stream_splits_exactly() {
        let mut stream = Vec::new();
        for i in 0..3u8 {
            let img = image::RgbaImage::from_pixel(10 + i as u32, 8, Rgba([i, 100, 50, 255]));
            let mut buf = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(img)
                .to_rgb8()
                .write_to(&mut buf, image::ImageFormat::Bmp)
                .unwrap();
            stream.extend_from_slice(buf.get_ref());
        }
        let parts = split_bmp_stream(&stream);
        assert_eq!(parts.len(), 3);
        for (i, p) in parts.iter().enumerate() {
            let img = image::load_from_memory(p).unwrap();
            assert_eq!(img.width(), 10 + i as u32);
        }
    }

    #[test]
    fn closed_loop_detected_and_sampled() {
        // 呼吸循环：大小 40→46→52→46→40（首尾同构 ⇒ 闭合）
        let sizes = [40u32, 46, 52, 46, 40, 46, 52, 46, 40];
        let frames: Vec<RgbaImage> = sizes.iter().map(|&s| blob_frame(80, s)).collect();
        let sel = select_frames(&frames, 6).unwrap();
        assert!(sel.loop_closed, "首尾同构应判定闭合");
        assert!(!sel.used_pingpong);
        assert!(sel.indices.len() >= 2 && sel.indices.len() <= 6);
    }

    #[test]
    fn open_sequence_uses_pingpong() {
        // 单调变化（40→75），首尾差异大 ⇒ 不闭合 ⇒ 正倒放
        let frames: Vec<RgbaImage> = (0..8)
            .map(|i| blob_frame(30 + i * 12, 40 + i * 5))
            .collect();
        let sel = select_frames(&frames, 6).unwrap();
        assert!(!sel.loop_closed);
        assert!(sel.used_pingpong);
    }

    #[test]
    fn unstable_frames_dropped() {
        // 一帧主体面积暴涨（镜头抖动/背景漂移）应被过滤；
        // 基准帧带呼吸变化，保证去重后仍有多帧可用
        let sizes = [40u32, 46, 52, 46, 40, 46];
        let mut frames: Vec<RgbaImage> = sizes.iter().map(|&s| blob_frame(80, s)).collect();
        frames.insert(3, blob_frame(10, 100));
        let sel = select_frames(&frames, 4).unwrap();
        assert!(sel.dropped_unstable >= 1);
    }

    #[test]
    fn all_static_frames_rejected() {
        let frames: Vec<RgbaImage> = (0..10).map(|_| blob_frame(80, 44)).collect();
        assert!(
            select_frames(&frames, 6).is_err(),
            "完全静止的视频应报错提示"
        );
    }

    /// 回归：细微待机动作绝不能被当成「静止」。
    ///
    /// 旧实现用 pHash 汉明距离做去重（阈值 5），而 pHash 是身份不变量——
    /// 对呼吸、眨眼这类形变几乎无响应。一段真实的豆包待机视频实测相邻帧距离恒为
    /// 0–2，于是 100 帧被去重成 1 帧，直接报「视频动作幅度过小」。
    /// 这里用同样量级的细微形变（每帧 1px）复现该场景。
    #[test]
    fn subtle_idle_motion_is_not_mistaken_for_static() {
        let sizes = [44u32, 45, 46, 45, 44, 45, 46, 45, 44, 45, 46, 45];
        let frames: Vec<RgbaImage> = sizes.iter().map(|&s| blob_frame(80, s)).collect();

        // 旧判据：pHash 对这种幅度基本无响应
        let hashes: Vec<u64> = frames.iter().map(phash::phash).collect();
        let max_ph = (1..hashes.len())
            .map(|i| phash::hamming(hashes[i - 1], hashes[i]))
            .max()
            .unwrap();
        assert!(
            max_ph < 5,
            "前提：pHash 对细微动作确实无响应（实测 {max_ph}）"
        );

        // 新判据：应当正常抽出动画
        let sel = select_frames(&frames, 6).expect("细微待机动作必须能抽出动画帧");
        assert!(
            sel.indices.len() >= 2,
            "至少要抽出 2 帧，实际 {}",
            sel.indices.len()
        );
    }

    /// 分段选帧：同一段素材按区间切开后，各段独立成动画。
    #[test]
    fn segment_ranges_select_independently() {
        /// 身体固定 + 一条腿前后摆动 —— 真正的姿态变化。
        /// （纯平移不算：入格时质心居中本来就会把它抹掉，见 motion 模块单测。）
        fn stride_frame(leg_dx: u32) -> RgbaImage {
            let mut img = blob_frame(80, 46);
            for y in 76..90 {
                for x in (70 + leg_dx)..(78 + leg_dx) {
                    if x < 200 && y < 120 {
                        img.put_pixel(x, y, Rgba([200, 90, 60, 255]));
                    }
                }
            }
            img
        }

        // 前半段呼吸（原地缩放），后半段迈步（腿部摆动）
        let mut frames: Vec<RgbaImage> = Vec::new();
        for s in [40u32, 46, 52, 46, 40, 46, 52, 46] {
            frames.push(blob_frame(80, s));
        }
        for dx in [0u32, 12, 24, 12, 0, 12, 24, 12] {
            frames.push(stride_frame(dx));
        }
        let areas: Vec<Option<f32>> = frames
            .iter()
            .map(|f| layout::bbox(f).map(|b| (b.w * b.h) as f32))
            .collect();
        let thumbs = motion::thumbs_from_cut(&frames);

        let a = select_from_analysis(&thumbs[0..8], &areas[0..8], 4).unwrap();
        let b = select_from_analysis(&thumbs[8..16], &areas[8..16], 4).unwrap();
        assert!(a.indices.iter().all(|&i| i < 8));
        assert!(b.indices.iter().all(|&i| i < 8));
        assert!(a.indices.len() >= 2 && b.indices.len() >= 2);
    }

    /// 端到端冒烟（需本机 ffmpeg）：合成帧 → mp4 → 抽帧。`cargo test -- --ignored`
    #[test]
    #[ignore]
    fn extract_frames_smoke() {
        let Some(ffmpeg) = crate::exporter::ffmpeg::locate(None) else {
            eprintln!("跳过：未找到 ffmpeg");
            return;
        };
        let dir = std::env::temp_dir().join("sf-pet-decoder-smoke");
        std::fs::create_dir_all(&dir).unwrap();
        let mp4 = dir.join("in.mp4");
        let mut enc = crate::exporter::ffmpeg::Encoder::start(&ffmpeg, 160, 120, 10, &mp4).unwrap();
        for i in 0..20u32 {
            let mut frame = image::RgbaImage::from_pixel(160, 120, Rgba([0, 255, 0, 255]));
            for y in 40..80 {
                for x in (30 + i)..(70 + i) {
                    frame.put_pixel(x, y, Rgba([200, 60, 60, 255]));
                }
            }
            enc.write_frame(frame.as_raw()).unwrap();
        }
        enc.finish().unwrap();

        let frames = extract_frames(&ffmpeg, &mp4, &ExtractOptions::default()).unwrap();
        assert!(
            frames.len() >= 10,
            "2 秒 @10fps 应抽出 ≥10 帧，实际 {}",
            frames.len()
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
