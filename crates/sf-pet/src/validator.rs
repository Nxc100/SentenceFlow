//! 校验器（方案 §8.4，转译自 hatch-pet 的 QA 思路）。
//!
//! 硬性（Error，阻断）：单元格几何、透明残留、声明帧数与实际一致、空帧、越格。
//! 软性（Warning，提示）：帧间 pHash 身份漂移、首帧可独立展示。

use image::RgbaImage;
use serde::Serialize;

use crate::imaging::{ALPHA_SOLID, layout, phash};
use crate::petkit::{
    CELL_H, CELL_W, GRID_COLS, GRID_ROWS, MAX_FRAMES, PetSpec, SHEET_H, SHEET_W, SPEC_VERSION,
    StateId,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Error,
    Warning,
    #[allow(dead_code)] // 预留给非告警级提示（前端已支持渲染）
    Info,
}

#[derive(Clone, Debug, Serialize)]
pub struct Finding {
    pub level: Level,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame: Option<u32>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub ok: bool,
    pub findings: Vec<Finding>,
}

impl Report {
    fn push(&mut self, level: Level, code: &str, message: impl Into<String>) {
        self.push_at(level, code, message, None, None);
    }

    fn push_at(
        &mut self,
        level: Level,
        code: &str,
        message: impl Into<String>,
        state: Option<String>,
        frame: Option<u32>,
    ) {
        if level == Level::Error {
            self.ok = false;
        }
        self.findings.push(Finding {
            level,
            code: code.into(),
            message: message.into(),
            state,
            frame,
        });
    }
}

pub fn validate(spec: &PetSpec, sheet: &RgbaImage) -> Report {
    let mut report = Report {
        ok: true,
        findings: Vec::new(),
    };

    // —— 规范与几何 ——
    if spec.spec != SPEC_VERSION {
        report.push(
            Level::Error,
            "spec-version",
            format!("不支持的规范版本 {}（当前支持 {SPEC_VERSION}）", spec.spec),
        );
    }
    if spec.cell.w != CELL_W || spec.cell.h != CELL_H {
        report.push(
            Level::Error,
            "cell-size",
            format!(
                "单元格必须为 {CELL_W}×{CELL_H}，实际 {}×{}",
                spec.cell.w, spec.cell.h
            ),
        );
    }
    if sheet.dimensions() != (SHEET_W, SHEET_H) {
        report.push(
            Level::Error,
            "sheet-size",
            format!(
                "图集必须为 {SHEET_W}×{SHEET_H}（8列×8行），实际 {}×{}",
                sheet.width(),
                sheet.height()
            ),
        );
        return report; // 几何不对，后续像素级检查无意义
    }

    if spec.states.is_empty() {
        report.push(Level::Error, "no-states", "至少需要一个状态（idle）");
    }
    if !spec.states.contains_key("idle") {
        report.push(
            Level::Error,
            "missing-idle",
            "缺少 idle 待机状态（运行时的默认状态）",
        );
    }

    // —— 状态声明 ——
    for (key, clip) in &spec.states {
        let sid = StateId::from_key(key);
        if sid.is_none() {
            report.push_at(
                Level::Warning,
                "unknown-state",
                format!("未知状态 {key}，运行时将忽略"),
                Some(key.clone()),
                None,
            );
            continue;
        }
        if clip.row >= GRID_ROWS {
            report.push_at(
                Level::Error,
                "row-out-of-range",
                format!("状态 {key} 行号 {} 超出图集（0–7）", clip.row),
                Some(key.clone()),
                None,
            );
            continue;
        }
        if clip.frames == 0 || clip.frames > MAX_FRAMES {
            report.push_at(
                Level::Error,
                "frame-count",
                format!("状态 {key} 帧数 {} 非法（1–{MAX_FRAMES}）", clip.frames),
                Some(key.clone()),
                None,
            );
            continue;
        }
        if !(1..=30).contains(&clip.fps) {
            report.push_at(
                Level::Warning,
                "fps-range",
                format!("状态 {key} fps={} 超出常规范围（1–30）", clip.fps),
                Some(key.clone()),
                None,
            );
        }
        if let Some(src) = &clip.mirror_of {
            if !spec.states.contains_key(src) {
                report.push_at(
                    Level::Error,
                    "mirror-missing",
                    format!("状态 {key} 镜像来源 {src} 不存在"),
                    Some(key.clone()),
                    None,
                );
            }
            continue; // 镜像状态不占实际像素
        }

        // —— 像素级：声明的每一帧非空、不越格贴边；行内多余单元格应为空 ——
        let mut hashes: Vec<u64> = Vec::new();
        for col in 0..GRID_COLS {
            let cell = layout::cell_at(sheet, clip.row, col);
            let content = layout::bbox(&cell);
            if col < clip.frames {
                match content {
                    None => report.push_at(
                        Level::Error,
                        "empty-frame",
                        format!("状态 {key} 第 {} 帧为空帧", col + 1),
                        Some(key.clone()),
                        Some(col),
                    ),
                    Some(b) => {
                        // 贴边=疑似越格被截断
                        if b.x0 == 0 || b.y0 == 0 || b.x0 + b.w >= CELL_W || b.y0 + b.h >= CELL_H {
                            report.push_at(
                                Level::Warning,
                                "touching-border",
                                format!(
                                    "状态 {key} 第 {} 帧内容贴到单元格边缘，疑似越格截断",
                                    col + 1
                                ),
                                Some(key.clone()),
                                Some(col),
                            );
                        }
                        hashes.push(phash::phash(&cell));
                    }
                }
            } else if content.is_some() {
                report.push_at(
                    Level::Error,
                    "extra-frame",
                    format!(
                        "状态 {key} 声明 {} 帧，但第 {} 格仍有内容（声明帧数与实际不一致）",
                        clip.frames,
                        col + 1
                    ),
                    Some(key.clone()),
                    Some(col),
                );
            }
        }

        // —— 软性：相邻帧 pHash 漂移 ——
        for i in 1..hashes.len() {
            let d = phash::hamming(hashes[i - 1], hashes[i]);
            if d > phash::DRIFT_THRESHOLD {
                report.push_at(
                    Level::Warning,
                    "identity-drift",
                    format!(
                        "状态 {key} 第 {} 帧与上一帧差异过大（pHash 距离 {d}），疑似身份漂移，建议救帧",
                        i + 1
                    ),
                    Some(key.clone()),
                    Some(i as u32),
                );
            }
        }
    }

    // —— 透明残留（硬性）：alpha=0 像素的 RGB 必须为 0 ——
    let residue = sheet
        .pixels()
        .filter(|p| p.0[3] == 0 && (p.0[0] != 0 || p.0[1] != 0 || p.0[2] != 0))
        .count();
    if residue > 0 {
        report.push(
            Level::Error,
            "transparent-residue",
            format!("检测到 {residue} 个透明像素带非零 RGB 残留（会产生描边脏色），请重新打包"),
        );
    }

    // —— 软性：首帧可作静态展示 ——
    if let Some(idle) = spec.states.get("idle") {
        if idle.mirror_of.is_none() && idle.row < GRID_ROWS {
            let first = layout::cell_at(sheet, idle.row, 0);
            let solid = first.pixels().filter(|p| p.0[3] >= ALPHA_SOLID).count();
            if solid < 40 {
                report.push(
                    Level::Warning,
                    "weak-first-frame",
                    "idle 首帧内容过少，托盘图标与减少动效模式将不可用",
                );
            }
        }
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::imaging::layout::{compose_sheet, fit_into_cell};
    use crate::petkit::StateClip;
    use image::Rgba;
    use std::collections::BTreeMap;

    fn subject(seed: u8) -> RgbaImage {
        let mut img = RgbaImage::new(120, 160);
        for y in 20..140 {
            for x in 20..100 {
                img.put_pixel(x, y, Rgba([200, seed, 100, 255]));
            }
        }
        img
    }

    fn make_pet(frames: u32) -> (PetSpec, RgbaImage) {
        let mut spec = PetSpec::new("test", "L1");
        spec.states.insert(
            "idle".into(),
            StateClip {
                row: 0,
                frames,
                fps: 8,
                looping: true,
                mirror_of: None,
                source: None,
            },
        );
        let mut rows = BTreeMap::new();
        let cells: Vec<RgbaImage> = (0..frames)
            .map(|i| fit_into_cell(&subject(50 + i as u8 * 3), 1.0))
            .collect();
        rows.insert(StateId::Idle, cells);
        (spec, compose_sheet(&rows))
    }

    #[test]
    fn valid_pet_passes() {
        let (spec, sheet) = make_pet(4);
        let r = validate(&spec, &sheet);
        assert!(r.ok, "findings: {:?}", r.findings);
    }

    #[test]
    fn declared_more_frames_than_present_fails() {
        let (mut spec, sheet) = make_pet(4);
        spec.states.get_mut("idle").unwrap().frames = 6;
        let r = validate(&spec, &sheet);
        assert!(!r.ok);
        assert!(r.findings.iter().any(|f| f.code == "empty-frame"));
    }

    #[test]
    fn extra_content_beyond_declared_fails() {
        let (mut spec, sheet) = make_pet(4);
        spec.states.get_mut("idle").unwrap().frames = 2;
        let r = validate(&spec, &sheet);
        assert!(!r.ok);
        assert!(r.findings.iter().any(|f| f.code == "extra-frame"));
    }

    #[test]
    fn wrong_sheet_size_fails() {
        let (spec, _) = make_pet(2);
        let bad = RgbaImage::new(100, 100);
        let r = validate(&spec, &bad);
        assert!(!r.ok);
        assert!(r.findings.iter().any(|f| f.code == "sheet-size"));
    }

    #[test]
    fn transparent_residue_fails() {
        let (spec, mut sheet) = make_pet(2);
        sheet.put_pixel(1500, 1600, Rgba([10, 20, 30, 0]));
        let r = validate(&spec, &sheet);
        assert!(!r.ok);
        assert!(r.findings.iter().any(|f| f.code == "transparent-residue"));
    }

    #[test]
    fn missing_idle_fails() {
        let (mut spec, sheet) = make_pet(2);
        let clip = spec.states.remove("idle").unwrap();
        spec.states.insert(
            "happy".into(),
            StateClip {
                row: 3,
                frames: clip.frames,
                ..clip
            },
        );
        let r = validate(&spec, &sheet);
        assert!(r.findings.iter().any(|f| f.code == "missing-idle"));
    }
}
