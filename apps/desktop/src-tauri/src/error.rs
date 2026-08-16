//! Command error type — serialized as `{ code, message }` so the frontend can
//! branch on `code` while showing `message` (already 人话, §11.E).

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct CmdError {
    pub code: &'static str,
    pub message: String,
}

impl CmdError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new("internal", message)
    }
}

impl std::fmt::Display for CmdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl From<rusqlite::Error> for CmdError {
    fn from(e: rusqlite::Error) -> Self {
        Self::new("db", e.to_string())
    }
}

impl From<sf_pipeline::store::StoreError> for CmdError {
    fn from(e: sf_pipeline::store::StoreError) -> Self {
        Self::new("db", e.to_string())
    }
}

impl From<std::io::Error> for CmdError {
    fn from(e: std::io::Error) -> Self {
        Self::new("io", e.to_string())
    }
}

impl From<serde_json::Error> for CmdError {
    fn from(e: serde_json::Error) -> Self {
        Self::new("json", e.to_string())
    }
}

pub type CmdResult<T> = Result<T, CmdError>;
