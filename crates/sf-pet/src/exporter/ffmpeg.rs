//! ffmpeg 定位与 rawvideo 管道编码（方案 §8.1：视频导出走 ffmpeg sidecar/系统安装）。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};

use crate::error::{PetError, PetResult};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn command(program: &Path) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// 定位 ffmpeg：设置覆盖 > 应用目录 sidecar > PATH。
pub fn locate(override_path: Option<&str>) -> Option<PathBuf> {
    if let Some(p) = override_path {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Some(p);
        }
    }
    let exe_name = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    if let Ok(current) = std::env::current_exe() {
        if let Some(dir) = current.parent() {
            let sidecar = dir.join(exe_name);
            if sidecar.is_file() {
                return Some(sidecar);
            }
        }
    }
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(exe_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// 验证可执行（跑 -version）。
pub fn probe(path: &Path) -> bool {
    command(path)
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// RGBA 裸帧 → H.264 MP4 流式编码器。
pub struct Encoder {
    child: Child,
    stdin: Option<ChildStdin>,
    frame_len: usize,
}

impl Encoder {
    pub fn start(ffmpeg: &Path, w: u32, h: u32, fps: u32, dest: &Path) -> PetResult<Encoder> {
        let mut child = command(ffmpeg)
            .args([
                "-y",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgba",
                "-s",
                &format!("{w}x{h}"),
                "-r",
                &fps.to_string(),
                "-i",
                "pipe:0",
                "-an",
                "-c:v",
                "libx264",
                "-preset",
                "veryfast",
                "-crf",
                "20",
                "-pix_fmt",
                "yuv420p",
                "-movflags",
                "+faststart",
            ])
            .arg(dest)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| PetError::msg(format!("无法启动 ffmpeg: {e}")))?;
        let stdin = child.stdin.take();
        Ok(Encoder {
            child,
            stdin,
            frame_len: (w * h * 4) as usize,
        })
    }

    pub fn write_frame(&mut self, rgba: &[u8]) -> PetResult<()> {
        debug_assert_eq!(rgba.len(), self.frame_len);
        self.stdin
            .as_mut()
            .ok_or_else(|| PetError::msg("编码器已关闭"))?
            .write_all(rgba)
            .map_err(|e| PetError::msg(format!("写入 ffmpeg 失败: {e}")))
    }

    pub fn finish(mut self) -> PetResult<()> {
        drop(self.stdin.take()); // 关闭管道让 ffmpeg 收尾
        let output = self
            .child
            .wait_with_output()
            .map_err(|e| PetError::msg(format!("等待 ffmpeg 失败: {e}")))?;
        if output.status.success() {
            Ok(())
        } else {
            let tail: String = String::from_utf8_lossy(&output.stderr)
                .lines()
                .rev()
                .take(4)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n");
            Err(PetError::msg(format!("ffmpeg 编码失败：{tail}")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 端到端编码冒烟：需要本机 ffmpeg，默认忽略（CI 无 ffmpeg 时不失败）。
    /// 运行：`cargo test -- --ignored`
    #[test]
    #[ignore]
    fn encode_smoke() {
        let Some(ffmpeg) = locate(None) else {
            eprintln!("跳过：未找到 ffmpeg");
            return;
        };
        assert!(probe(&ffmpeg));
        let dest = std::env::temp_dir().join("sf-pet-encode-smoke.mp4");
        let (w, h, fps) = (320u32, 240u32, 30u32);
        let mut enc = Encoder::start(&ffmpeg, w, h, fps, &dest).unwrap();
        let frame = vec![200u8; (w * h * 4) as usize];
        for _ in 0..30 {
            enc.write_frame(&frame).unwrap();
        }
        enc.finish().unwrap();
        let size = std::fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
        assert!(size > 500, "mp4 应有实际内容，实际 {size} 字节");
        std::fs::remove_file(&dest).ok();
    }
}
