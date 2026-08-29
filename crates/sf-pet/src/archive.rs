//! `.petkit` 宠物包(zip:pet.json + spritesheet)导入/导出。
//!
//! 导入前双层校验:先按 PetKit 契约解出 `PetSpec` 与图集,再跑
//! [`crate::validator`] 的硬性规则;任一不过即拒绝落盘。

use std::io::{Cursor, Read, Write};
use std::path::Path;

use zip::write::SimpleFileOptions;

use crate::error::{PetError, PetResult};
use crate::petkit::PetSpec;
use crate::store;
use crate::validator;

/// 导出 `.petkit` 文件到 `dest`。
pub fn export_petkit(pets_root: &Path, pet_id: &str, dest: &Path) -> PetResult<()> {
    let spec = store::load_spec(pets_root, pet_id)?;
    let (sheet_bytes, _) = store::load_sheet_bytes(pets_root, pet_id)?;

    let file = std::fs::File::create(dest)?;
    let mut zip = zip::ZipWriter::new(file);
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    zip.start_file("pet.json", opts)?;
    zip.write_all(serde_json::to_string_pretty(&spec)?.as_bytes())?;
    // sheet 已是压缩格式,直接存储
    let stored = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    zip.start_file(&spec.sheet, stored)?;
    zip.write_all(&sheet_bytes)?;
    zip.finish()?;
    Ok(())
}

/// 从 `.petkit` 包里解出 spec 与图集(不落盘)。导入与预检共用这一步。
pub fn read_petkit(src: &Path) -> PetResult<(PetSpec, image::RgbaImage)> {
    let file = std::fs::File::open(src).map_err(|_| PetError::msg("无法打开 .petkit 文件"))?;
    let mut zip = zip::ZipArchive::new(file)?;

    let mut spec_raw = String::new();
    zip.by_name("pet.json")
        .map_err(|_| PetError::msg("包内缺少 pet.json,不是有效的宠物包"))?
        .read_to_string(&mut spec_raw)?;
    let spec: PetSpec = serde_json::from_str(spec_raw.trim_start_matches('\u{feff}'))
        .map_err(|e| PetError::msg(format!("pet.json 解析失败: {e}")))?;

    let mut sheet_bytes = Vec::new();
    zip.by_name(&spec.sheet)
        .map_err(|_| PetError::msg(format!("包内缺少图集文件 {}", spec.sheet)))?
        .read_to_end(&mut sheet_bytes)?;

    let sheet = image::ImageReader::new(Cursor::new(&sheet_bytes))
        .with_guessed_format()?
        .decode()
        .map_err(|e| PetError::msg(format!("图集解码失败: {e}")))?
        .to_rgba8();
    Ok((spec, sheet))
}

/// 导入前预检:解包 → 校验,返回报告(供 UI 展示,不落盘)。
pub fn validate_petkit_file(src: &Path) -> PetResult<validator::Report> {
    let (spec, sheet) = read_petkit(src)?;
    Ok(validator::validate(&spec, &sheet))
}

/// 导入 `.petkit`:解包 → 校验(硬性规则必须通过)→ 存入宠物库,返回新 ID。
pub fn import_petkit(pets_root: &Path, seq: u32, src: &Path) -> PetResult<String> {
    let (spec, sheet) = read_petkit(src)?;

    let report = validator::validate(&spec, &sheet);
    if !report.ok {
        let msgs: Vec<String> = report
            .findings
            .iter()
            .filter(|f| f.level == validator::Level::Error)
            .map(|f| f.message.clone())
            .collect();
        return Err(PetError::msg(format!(
            "宠物包未通过校验:{}",
            msgs.join(";")
        )));
    }

    store::save_pet(pets_root, seq, &spec, &sheet)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::petkit::{CellSize, StateClip};

    fn tiny_pet() -> (PetSpec, image::RgbaImage) {
        let mut spec = PetSpec {
            spec: crate::petkit::SPEC_VERSION.into(),
            name: "打包测试".into(),
            author: "local".into(),
            cell: CellSize {
                w: crate::petkit::CELL_W,
                h: crate::petkit::CELL_H,
            },
            sheet: "spritesheet.webp".into(),
            chroma: None,
            states: std::collections::BTreeMap::new(),
            tier: Some("L0".into()),
            created: None,
            anchors: None,
        };
        spec.states.insert(
            "idle".into(),
            StateClip {
                row: 0,
                frames: 1,
                fps: 2,
                looping: true,
                mirror_of: None,
                source: None,
            },
        );
        // 图集必须是契约尺寸,且 idle 首格要有内容,否则校验不过。
        let mut sheet = image::RgbaImage::new(crate::petkit::SHEET_W, crate::petkit::SHEET_H);
        for y in 40..180 {
            for x in 40..150 {
                sheet.put_pixel(x, y, image::Rgba([180, 120, 90, 255]));
            }
        }
        (spec, sheet)
    }

    /// 导出 → 导入的完整往返:包能被自己读回,且校验通过后落进宠物库。
    #[test]
    fn export_then_import_round_trip() {
        let root = std::env::temp_dir().join("sf-pet-archive-round-trip");
        let _ = std::fs::remove_dir_all(&root);
        store::ensure_root(&root).unwrap();

        let (spec, sheet) = tiny_pet();
        let src_id = store::save_pet(&root, 0, &spec, &sheet).unwrap();

        let kit = root.join("out.petkit");
        export_petkit(&root, &src_id, &kit).unwrap();
        assert!(validate_petkit_file(&kit).unwrap().ok, "自产包应自检通过");

        let new_id = import_petkit(&root, 1, &kit).unwrap();
        assert_ne!(new_id, src_id, "导入应产生新宠物");
        assert_eq!(store::load_spec(&root, &new_id).unwrap().name, "打包测试");
        std::fs::remove_dir_all(&root).ok();
    }

    /// 不是 zip / 缺 pet.json 的文件必须给出人话提示,而不是把裸错误抛给用户。
    #[test]
    fn rejects_files_that_are_not_petkits() {
        let dir = std::env::temp_dir().join("sf-pet-archive-bad");
        std::fs::create_dir_all(&dir).unwrap();
        let bogus = dir.join("not-a-pet.petkit");
        std::fs::write(&bogus, b"definitely not a zip").unwrap();
        assert!(read_petkit(&bogus).is_err());
        assert!(read_petkit(&dir.join("missing.petkit")).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
