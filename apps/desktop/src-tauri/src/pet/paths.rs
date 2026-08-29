//! AI 萌宠的数据位置。
//!
//! 全部收在句流数据根的 `pet/` 子目录下,与既有的 progress.db / user_content.db
//! 平级但互不干扰:
//!
//! ```text
//! <app-data>/pet/
//! ├── pets/<petId>/{pet.json, spritesheet.webp, rig_source.png, source/<state>/NNN.png}
//! └── reminders.json
//! ```
//!
//! 备份契约不变:`backup_export` 仍只打包 progress.db 与 user_content.db,
//! 宠物素材**不进**备份包(体积量级完全不同,且可由 .petkit 单独导出)。

use std::path::{Path, PathBuf};

pub struct PetPaths {
    pub root: PathBuf,
}

impl PetPaths {
    pub fn new(app_data_root: &Path) -> Self {
        Self {
            root: app_data_root.join("pet"),
        }
    }

    /// 宠物库根目录(传给 `sf_pet::store` 的那个 `pets_root`)。
    pub fn pets_dir(&self) -> PathBuf {
        self.root.join("pets")
    }

    pub fn reminders_file(&self) -> PathBuf {
        self.root.join("reminders.json")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 宠物数据必须整体待在 `pet/` 子目录里,不得散落到句流数据根。
    #[test]
    fn everything_lives_under_the_pet_subdir() {
        let paths = PetPaths::new(Path::new("/data"));
        assert_eq!(paths.root, PathBuf::from("/data/pet"));
        for p in [paths.pets_dir(), paths.reminders_file()] {
            assert!(p.starts_with("/data/pet"), "{p:?} 逃出了 pet/ 子目录");
        }
    }
}
