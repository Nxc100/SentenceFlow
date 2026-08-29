//! `sf-pet` —— AI 萌宠模块的纯逻辑内核。
//!
//! 架构不变量(《AI 萌宠模块-整合执行方案》§3.1):
//! * **零 tauri 依赖**:本 crate 不知道窗口、命令、事件的存在;所有落盘位置由
//!   调用方以 `&Path` 传入,所有进度由调用方以回调注入。
//! * **双向隔离**:不依赖任何句流 crate(`sf-core`/`sf-pipeline`/`sf-llm`),
//!   句流练习路径也不 import 本 crate。
//!
//! 模块地图:
//!
//! | 模块 | 职责 |
//! |---|---|
//! | [`petkit`] | PetKit v0.1 契约:8 状态、图集几何、`PetSpec` |
//! | [`store`] | 宠物库落盘:`<pets_root>/<id>/{pet.json, spritesheet.webp, source/}` |
//! | [`archive`] | `.petkit` 包(zip)导入导出 |
//! | [`imaging`] | 抠背景 / 降噪 / 切帧 / 入格 / pHash / 视频解帧 / 运动分析 / 语义抠图 |
//! | [`rig`] | 骨骼烘焙:轮廓 → 网格 → ARAP → 步态 → 软光栅 |
//! | [`validator`] | 图集硬性 + 软性双层校验 |
//! | [`exporter`] | 出生视频逐帧渲染 + ffmpeg 管道 |
//! | [`spellbook`] | 咒语包模板拼装 + 程序化引导图 |
//! | [`wizard`] | 孵化 / 进化编排(会话 → 合成 → 校验 → 落盘) |

pub mod archive;
pub mod error;
pub mod exporter;
pub mod imaging;
pub mod petkit;
pub mod rig;
pub mod spellbook;
pub mod store;
pub mod validator;
pub mod wizard;

pub use error::{PetError, PetResult};

/// 读 JSON 文本并剥离 UTF-8 BOM。
///
/// 记事本等编辑器保存的文件常带 BOM,而 `serde_json` 不接受 —— 用户手工编辑过
/// `pet.json` / `reminders.json` 后本该照常读入,不该报「数据格式错误」。
pub fn read_json_text(path: &std::path::Path) -> std::io::Result<String> {
    let raw = std::fs::read_to_string(path)?;
    Ok(raw.trim_start_matches('\u{feff}').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_json_text_strips_bom() {
        let dir = std::env::temp_dir().join("sf-pet-bom");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("with-bom.json");
        std::fs::write(&path, "\u{feff}{\"a\":1}").unwrap();
        let raw = read_json_text(&path).unwrap();
        assert!(raw.starts_with('{'), "BOM 未剥离: {raw:?}");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&raw).unwrap()["a"],
            1
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
