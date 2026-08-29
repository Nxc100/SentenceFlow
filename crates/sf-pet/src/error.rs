//! 内核错误类型。
//!
//! 与句流桌面壳的 `CmdError` 刻意分开:本 crate 零 tauri 依赖,错误文案已是
//! 面向用户的中文(与句流 §11.E「错误即人话」同一约定),由胶水层转译成
//! `{ code, message }` 交给前端。

/// AI 萌宠内核错误。
#[derive(Debug, thiserror::Error)]
pub enum PetError {
    /// 已经写好的中文提示,直接展示给用户。
    #[error("{0}")]
    Msg(String),
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
    #[error("图片处理失败: {0}")]
    Image(#[from] image::ImageError),
    #[error("数据格式错误: {0}")]
    Json(#[from] serde_json::Error),
    #[error("压缩包错误: {0}")]
    Zip(#[from] zip::result::ZipError),
}

impl PetError {
    pub fn msg(s: impl Into<String>) -> Self {
        PetError::Msg(s.into())
    }
}

pub type PetResult<T> = Result<T, PetError>;
