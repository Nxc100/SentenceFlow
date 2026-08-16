//! Application data locations (spec §7.7).
//!
//! ```text
//! <app-data>/
//! ├── progress.db          learning progress (never shipped, always backed up)
//! ├── user_content.db      generated/imported sentence sets
//! ├── license.sflic        activated license (user-copyable, 换机零摩擦)
//! ├── trial.json           trial anchor (file half; keyring holds the twin)
//! ├── channels.json        latest channel policy (content-pack refreshed)
//! └── agent-sandbox/       cwd for the opencode agent (our opencode.json)
//! ```

use std::path::PathBuf;

pub struct AppPaths {
    pub root: PathBuf,
}

impl AppPaths {
    pub fn new(root: PathBuf) -> std::io::Result<Self> {
        std::fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    pub fn progress_db(&self) -> PathBuf {
        self.root.join("progress.db")
    }

    pub fn user_content_db(&self) -> PathBuf {
        self.root.join("user_content.db")
    }

    pub fn license_file(&self) -> PathBuf {
        self.root.join("license.sflic")
    }

    pub fn trial_file(&self) -> PathBuf {
        self.root.join("trial.json")
    }

    pub fn channels_json(&self) -> PathBuf {
        self.root.join("channels.json")
    }

    pub fn agent_sandbox(&self) -> PathBuf {
        self.root.join("agent-sandbox")
    }

    /// Shipped read-only content.db: resolved from the Tauri resource dir in
    /// production; a repo-relative fallback keeps `cargo run`/dev working.
    pub fn bundled_content_db(resource_dir: Option<PathBuf>) -> Option<PathBuf> {
        let mut candidates = Vec::new();
        if let Some(res) = resource_dir {
            candidates.push(res.join("content.db"));
        }
        candidates.push(PathBuf::from("content/build/content.db"));
        candidates.push(PathBuf::from("../../content/build/content.db"));
        candidates.into_iter().find(|p| p.exists())
    }
}
