//! 宠物库落盘:`<pets_root>/<id>/pet.json + spritesheet.webp + source/<state>/NNN.png`。
//!
//! 与 HatchDesk 原实现的唯一差异:所有位置由调用方以 `pets_root: &Path` 传入
//! (原来是 `AppHandle` 现取 `%APPDATA%`)。本模块因此可在临时目录里完整单测,
//! 也让「宠物数据收进 `<app_data>/pet/`」这一决策落在胶水层而非内核。

use std::path::{Path, PathBuf};

use image::RgbaImage;
use serde::Serialize;

use crate::error::{PetError, PetResult};
use crate::petkit::PetSpec;

/// 宠物 ID 里禁止出现的字符:目录穿越与扩展名混淆的唯一防线。
const ID_FORBIDDEN: [char; 3] = ['/', '\\', '.'];

#[derive(Clone, Serialize)]
pub struct PetMeta {
    pub id: String,
    pub name: String,
    pub tier: String,
    pub states: Vec<String>,
    pub created: Option<String>,
}

/// 生成宠物 ID。
///
/// `seq` 由调用方持有(胶水层放在 `PetState` 里),同一秒内连续孵化不会撞号。
/// 原实现用进程级 `static AtomicU32`;句流语境下进程是共享的,把序号收进状态
/// 更干净,也让本函数保持纯粹、可测。
pub fn new_pet_id(seq: u32) -> String {
    format!(
        "p{}{:02}",
        chrono::Local::now().format("%Y%m%d%H%M%S"),
        seq % 100
    )
}

/// 确保宠物库根目录存在。
pub fn ensure_root(pets_root: &Path) -> PetResult<()> {
    std::fs::create_dir_all(pets_root)?;
    Ok(())
}

pub fn pet_dir(pets_root: &Path, id: &str) -> PetResult<PathBuf> {
    if id.is_empty() || id.contains(ID_FORBIDDEN) {
        return Err(PetError::msg("非法宠物 ID"));
    }
    Ok(pets_root.join(id))
}

pub fn spec_path(pets_root: &Path, id: &str) -> PetResult<PathBuf> {
    Ok(pet_dir(pets_root, id)?.join("pet.json"))
}

pub fn load_spec(pets_root: &Path, id: &str) -> PetResult<PetSpec> {
    let raw = crate::read_json_text(&spec_path(pets_root, id)?)
        .map_err(|_| PetError::msg(format!("宠物 {id} 不存在或已损坏")))?;
    let mut spec: PetSpec = serde_json::from_str(&raw)?;
    if let Ok(dir) = pet_dir(pets_root, id) {
        backfill_sources_from(&dir.join("source"), &mut spec);
    }
    Ok(spec)
}

/// 回填 `StateClip::source`(旧档没有这个字段)。
///
/// 判据:只有向导导入(视频/网格/单条)会把源帧留在 `source/<state>/`,
/// 骨骼烘焙不会。因此留有源帧 ⇒ 真素材。回填只在内存里做,
/// 下次 `overwrite_pet` 时自然落盘,老宠物无需重新导入即可正确播放走路。
fn backfill_sources_from(root: &Path, spec: &mut PetSpec) {
    if !root.exists() {
        return;
    }
    // 先算出各状态是否留有源帧(镜像态不占素材,稍后从本体继承)
    let mut real: Vec<String> = Vec::new();
    for (key, clip) in spec.states.iter() {
        if clip.source.is_some() || clip.mirror_of.is_some() {
            continue;
        }
        let has_frames = std::fs::read_dir(root.join(key))
            .map(|mut d| d.next().is_some())
            .unwrap_or(false);
        if has_frames {
            real.push(key.clone());
        }
    }
    for key in real {
        if let Some(c) = spec.states.get_mut(&key) {
            c.source = Some(crate::petkit::SOURCE_REAL.into());
        }
    }
    // 镜像态继承本体来源
    let inherited: Vec<(String, Option<String>)> = spec
        .states
        .iter()
        .filter(|(_, c)| c.source.is_none())
        .filter_map(|(k, c)| {
            c.mirror_of
                .as_ref()
                .and_then(|src| spec.states.get(src))
                .map(|s| (k.clone(), s.source.clone()))
        })
        .collect();
    for (key, src) in inherited {
        if let Some(c) = spec.states.get_mut(&key) {
            c.source = src;
        }
    }
}

pub fn sheet_path(pets_root: &Path, id: &str) -> PetResult<PathBuf> {
    let spec = load_spec(pets_root, id)?;
    Ok(pet_dir(pets_root, id)?.join(&spec.sheet))
}

pub fn load_sheet(pets_root: &Path, id: &str) -> PetResult<RgbaImage> {
    let path = sheet_path(pets_root, id)?;
    let img = image::open(&path)?;
    Ok(img.to_rgba8())
}

pub fn load_sheet_bytes(pets_root: &Path, id: &str) -> PetResult<(Vec<u8>, String)> {
    let path = sheet_path(pets_root, id)?;
    let bytes = std::fs::read(&path)?;
    let mime = match path.extension().and_then(|e| e.to_str()) {
        Some("png") => "image/png",
        _ => "image/webp",
    };
    Ok((bytes, mime.into()))
}

/// 保存新宠物(sheet 以无损 WebP 落盘),返回宠物 ID。
pub fn save_pet(
    pets_root: &Path,
    seq: u32,
    spec: &PetSpec,
    sheet: &RgbaImage,
) -> PetResult<String> {
    let id = new_pet_id(seq);
    let dir = pet_dir(pets_root, &id)?;
    std::fs::create_dir_all(&dir)?;

    let mut spec = spec.clone();
    spec.sheet = "spritesheet.webp".into();
    let webp = crate::imaging::encode_webp(sheet)?;
    std::fs::write(dir.join(&spec.sheet), webp)?;
    std::fs::write(dir.join("pet.json"), serde_json::to_string_pretty(&spec)?)?;
    Ok(id)
}

/// 原地覆盖更新(进化):宠物 ID 不变,sheet 与 spec 整体重写。
pub fn overwrite_pet(
    pets_root: &Path,
    id: &str,
    spec: &PetSpec,
    sheet: &RgbaImage,
) -> PetResult<()> {
    let dir = pet_dir(pets_root, id)?;
    if !dir.exists() {
        return Err(PetError::msg(format!("宠物 {id} 不存在,无法进化")));
    }
    let mut spec = spec.clone();
    spec.sheet = "spritesheet.webp".into();
    let webp = crate::imaging::encode_webp(sheet)?;
    std::fs::write(dir.join(&spec.sheet), webp)?;
    std::fs::write(dir.join("pet.json"), serde_json::to_string_pretty(&spec)?)?;
    Ok(())
}

/// 仅重写 pet.json(不动 sheet)——认主校准写锚点等元数据更新用。
pub fn save_spec(pets_root: &Path, id: &str, spec: &PetSpec) -> PetResult<()> {
    let dir = pet_dir(pets_root, id)?;
    if !dir.exists() {
        return Err(PetError::msg(format!("宠物 {id} 不存在")));
    }
    std::fs::write(dir.join("pet.json"), serde_json::to_string_pretty(spec)?)?;
    Ok(())
}

/// 骨骼烘焙的源帧备份路径(保证重烘幂等、不在已烘焙帧上再烘)。
pub fn rig_source_path(pets_root: &Path, id: &str) -> PetResult<PathBuf> {
    Ok(pet_dir(pets_root, id)?.join("rig_source.png"))
}

// ---------------------------------------------------------------- 素材源帧
// 进化模式要求跨次导入时全局缩放一致:把每个状态处理后的原始帧持久化在
// pets/<id>/source/<state>/NNN.png,重合成时全量参与 global_scale。

fn source_state_dir(pets_root: &Path, id: &str, state_key: &str) -> PetResult<PathBuf> {
    if state_key.is_empty() || state_key.contains(ID_FORBIDDEN) {
        return Err(PetError::msg("非法状态名"));
    }
    Ok(pet_dir(pets_root, id)?.join("source").join(state_key))
}

pub fn save_source_frames(
    pets_root: &Path,
    id: &str,
    state_key: &str,
    frames: &[RgbaImage],
) -> PetResult<()> {
    let dir = source_state_dir(pets_root, id, state_key)?;
    if dir.exists() {
        std::fs::remove_dir_all(&dir)?;
    }
    std::fs::create_dir_all(&dir)?;
    for (i, frame) in frames.iter().enumerate() {
        let png = crate::imaging::encode_png(frame)?;
        std::fs::write(dir.join(format!("{i:03}.png")), png)?;
    }
    Ok(())
}

/// 读取全部素材源帧;无 source 目录的状态回退为从现有图集裁出单元格
/// (旧版宠物/导入宠物的兼容路径)。
pub fn load_all_source_frames(
    pets_root: &Path,
    id: &str,
) -> PetResult<std::collections::BTreeMap<String, Vec<RgbaImage>>> {
    use crate::imaging::layout;
    let spec = load_spec(pets_root, id)?;
    let mut out = std::collections::BTreeMap::new();
    let source_root = pet_dir(pets_root, id)?.join("source");
    let mut sheet_cache: Option<RgbaImage> = None;

    for (key, clip) in &spec.states {
        if clip.mirror_of.is_some() {
            continue; // 镜像态不占素材
        }
        let dir = source_root.join(key);
        let mut frames = Vec::new();
        if dir.is_dir() {
            let mut entries: Vec<_> = std::fs::read_dir(&dir)?
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("png"))
                .collect();
            entries.sort();
            for path in entries {
                if let Ok(img) = image::open(&path) {
                    frames.push(img.to_rgba8());
                }
            }
        }
        if frames.is_empty() {
            // 兼容回退:从图集裁单元格(已入格帧,参与重合成仍保持比例)
            let sheet = match sheet_cache {
                Some(ref s) => s,
                None => sheet_cache.insert(load_sheet(pets_root, id)?),
            };
            for col in 0..clip.frames {
                frames.push(layout::cell_at(sheet, clip.row, col));
            }
        }
        if !frames.is_empty() {
            out.insert(key.clone(), frames);
        }
    }
    Ok(out)
}

pub fn delete_pet(pets_root: &Path, id: &str) -> PetResult<()> {
    let dir = pet_dir(pets_root, id)?;
    if dir.exists() {
        std::fs::remove_dir_all(dir)?;
    }
    Ok(())
}

pub fn list_pets(pets_root: &Path) -> PetResult<Vec<PetMeta>> {
    ensure_root(pets_root)?;
    let mut out = Vec::new();
    for entry in std::fs::read_dir(pets_root)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let id = entry.file_name().to_string_lossy().to_string();
        match load_spec(pets_root, &id) {
            Ok(spec) => out.push(PetMeta {
                id,
                name: spec.name.clone(),
                tier: spec.effective_tier(),
                states: spec.states.keys().cloned().collect(),
                created: spec.created.clone(),
            }),
            Err(_) => continue, // 损坏目录不阻塞列表
        }
    }
    out.sort_by(|a, b| b.id.cmp(&a.id));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::petkit::{CellSize, SOURCE_REAL, StateClip};

    fn spec_with_walk() -> PetSpec {
        let mut spec = PetSpec {
            spec: crate::petkit::SPEC_VERSION.into(),
            name: "回填测试".into(),
            author: "local".into(),
            cell: CellSize {
                w: crate::petkit::CELL_W,
                h: crate::petkit::CELL_H,
            },
            sheet: "spritesheet.webp".into(),
            chroma: None,
            states: std::collections::BTreeMap::new(),
            tier: Some("L1".into()),
            created: None,
            anchors: None,
        };
        spec.states.insert(
            "idle".into(),
            StateClip {
                row: 0,
                frames: 6,
                fps: 8,
                looping: true,
                mirror_of: None,
                source: None,
            },
        );
        spec.states.insert(
            "walk-right".into(),
            StateClip {
                row: 1,
                frames: 6,
                fps: 10,
                looping: true,
                mirror_of: None,
                source: None,
            },
        );
        spec.states.insert(
            "walk-left".into(),
            StateClip {
                row: 2,
                frames: 6,
                fps: 10,
                looping: true,
                mirror_of: Some("walk-right".into()),
                source: None,
            },
        );
        spec
    }

    /// 旧档没有 `source` 字段:留有源帧的状态应回填为真素材,镜像态继承本体。
    /// 这是「老宠物无需重新导入也能正确播放真步态」的依据。
    #[test]
    fn backfills_real_from_persisted_source_frames() {
        let dir = std::env::temp_dir().join("sf-pet-backfill-real");
        let _ = std::fs::remove_dir_all(&dir);
        for key in ["idle", "walk-right"] {
            std::fs::create_dir_all(dir.join(key)).unwrap();
            std::fs::write(dir.join(key).join("000.png"), b"stub").unwrap();
        }

        let mut spec = spec_with_walk();
        backfill_sources_from(&dir, &mut spec);
        assert_eq!(spec.states["idle"].source.as_deref(), Some(SOURCE_REAL));
        assert_eq!(
            spec.states["walk-right"].source.as_deref(),
            Some(SOURCE_REAL)
        );
        assert_eq!(
            spec.states["walk-left"].source.as_deref(),
            Some(SOURCE_REAL),
            "镜像态应继承本体来源"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 骨骼烘焙不留源帧:没有 source 目录时不应误判成真素材
    /// (否则烘焙出的 walk 会绕过 cutout,香蕉弯/滑步回归)。
    #[test]
    fn baked_pet_without_source_frames_stays_unmarked() {
        let dir = std::env::temp_dir().join("sf-pet-backfill-baked");
        let _ = std::fs::remove_dir_all(&dir);
        let mut spec = spec_with_walk();
        backfill_sources_from(&dir, &mut spec);
        assert!(
            spec.states.values().all(|c| c.source.is_none()),
            "无源帧不应回填"
        );
    }

    /// 空目录也算没有素材。
    #[test]
    fn empty_state_dir_is_not_real() {
        let dir = std::env::temp_dir().join("sf-pet-backfill-empty");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("walk-right")).unwrap();
        let mut spec = spec_with_walk();
        backfill_sources_from(&dir, &mut spec);
        assert!(spec.states["walk-right"].source.is_none());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// 目录穿越防线:宠物 ID / 状态名不得逃出宠物库根目录。
    #[test]
    fn rejects_ids_that_escape_the_root() {
        let root = Path::new("/tmp/sf-pet-root");
        for bad in ["", "..", "a/b", "a\\b", "x.json"] {
            assert!(pet_dir(root, bad).is_err(), "{bad:?} 应被拒绝");
        }
        assert!(pet_dir(root, "p2026010112000001").is_ok());
        assert!(source_state_dir(root, "p1", "../../etc").is_err());
        assert!(source_state_dir(root, "p1", "walk-right").is_ok());
    }

    /// 落盘 → 列表 → 读回的完整往返(纯目录参数化后可直接单测)。
    #[test]
    fn save_list_and_delete_round_trip() {
        let root = std::env::temp_dir().join("sf-pet-store-round-trip");
        let _ = std::fs::remove_dir_all(&root);
        ensure_root(&root).unwrap();

        let mut spec = spec_with_walk();
        spec.name = "小咪".into();
        let sheet = RgbaImage::new(crate::petkit::SHEET_W, crate::petkit::SHEET_H);
        let id = save_pet(&root, 0, &spec, &sheet).unwrap();

        let listed = list_pets(&root).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "小咪");
        assert_eq!(load_spec(&root, &id).unwrap().name, "小咪");

        delete_pet(&root, &id).unwrap();
        assert!(list_pets(&root).unwrap().is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    /// 库根目录还不存在时,列表应是空清单而不是 IO 错误
    /// (全新用户第一次打开「我的宠物」走的就是这条路径)。
    #[test]
    fn listing_a_missing_root_yields_empty() {
        let root = std::env::temp_dir().join("sf-pet-store-missing-root");
        let _ = std::fs::remove_dir_all(&root);
        assert!(list_pets(&root).unwrap().is_empty());
        std::fs::remove_dir_all(&root).ok();
    }
}
