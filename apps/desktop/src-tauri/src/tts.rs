//! Offline TTS via a piper sidecar (spec §7.1 语音).
//!
//! The piper binary + voice models are packaged as Tauri resources
//! (`resources/piper/`). When they are absent (dev builds, or before the
//! audio pack lands) the command returns `None` and the frontend falls back
//! to the WebView's `speechSynthesis` — practice never blocks on audio.

use std::path::PathBuf;
use std::process::Stdio;

pub struct PiperTts {
    bin: PathBuf,
    voice_gb: PathBuf,
    voice_us: PathBuf,
    out_dir: PathBuf,
}

impl PiperTts {
    /// Detect a piper installation under the resource dir.
    pub fn detect(resource_dir: Option<PathBuf>, out_dir: PathBuf) -> Option<Self> {
        let base = resource_dir?.join("piper");
        let bin = if cfg!(windows) {
            base.join("piper.exe")
        } else {
            base.join("piper")
        };
        let voice_gb = base.join("en_GB-alba-medium.onnx");
        let voice_us = base.join("en_US-lessac-medium.onnx");
        (bin.exists() && voice_gb.exists()).then_some(Self {
            bin,
            voice_gb,
            voice_us,
            out_dir,
        })
    }

    /// Synthesize `text` to a wav file; returns its path.
    pub async fn speak(&self, text: &str, us_accent: bool, rate: f32) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(&self.out_dir)?;
        let voice = if us_accent && self.voice_us.exists() {
            &self.voice_us
        } else {
            &self.voice_gb
        };
        // Stable name per (text, voice, rate) so repeated playback hits cache.
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in text
            .as_bytes()
            .iter()
            .chain(voice.to_string_lossy().as_bytes())
        {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0000_0100_0000_01B3);
        }
        let out = self
            .out_dir
            .join(format!("tts-{h:016x}-{}.wav", (rate * 100.0) as u32));
        if out.exists() {
            return Ok(out);
        }
        // piper reads text on stdin; --length_scale is inverse speed.
        let length_scale = (1.0 / rate.clamp(0.6, 1.4)).to_string();
        let mut child = tokio::process::Command::new(&self.bin)
            .args([
                "--model",
                &voice.to_string_lossy(),
                "--output_file",
                &out.to_string_lossy(),
                "--length_scale",
                &length_scale,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        use tokio::io::AsyncWriteExt;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(text.as_bytes()).await?;
            stdin.shutdown().await?;
        }
        let status = child.wait().await?;
        if !status.success() {
            return Err(std::io::Error::other(format!("piper exited with {status}")));
        }
        Ok(out)
    }
}
