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
    /// production; repo-relative fallbacks keep `cargo run`/dev working.
    pub fn bundled_content_db(resource_dir: Option<PathBuf>) -> Option<PathBuf> {
        let candidates = Self::content_db_candidates(
            resource_dir,
            std::env::current_exe().ok(),
            Some(".".into()),
        );
        candidates.into_iter().find(|p| p.exists())
    }

    /// 候选路径清单(纯函数,便于测试)。按"越确定越靠前"排序。
    ///
    /// 除了工作目录,还要从 **exe 自身位置**往上找:工作目录取决于怎么启动的
    /// —— 双击是 exe 所在目录,快捷方式或在别处开终端跑就是别的地方,
    /// 于是 `target/release/sentenceflow-desktop.exe` 换个方式启动就找不到
    /// 句库,界面上空空如也却不报错。exe 位置是它自己带着的,跟怎么启动无关。
    fn content_db_candidates(
        resource_dir: Option<PathBuf>,
        current_exe: Option<PathBuf>,
        cwd: Option<PathBuf>,
    ) -> Vec<PathBuf> {
        // 相对仓库根的位置;从 exe 所在目录往上退几层就是仓库根:
        // <repo>/target/release/x.exe → 退 2 层;<repo>/target/debug/x.exe 同理。
        const REL: &str = "content/build/content.db";
        let mut out = Vec::new();

        if let Some(res) = resource_dir {
            out.push(res.join("content.db"));
        }
        if let Some(exe_dir) = current_exe.as_ref().and_then(|p| p.parent()) {
            // 打包安装后 content.db 与 exe 同级
            out.push(exe_dir.join("content.db"));
            // 开发构建:从 target/<profile>/ 退回仓库根
            let mut up = exe_dir.to_path_buf();
            for _ in 0..3 {
                out.push(up.join(REL));
                match up.parent() {
                    Some(p) => up = p.to_path_buf(),
                    None => break,
                }
            }
        }
        if let Some(cwd) = cwd {
            out.push(cwd.join(REL));
            out.push(cwd.join("../..").join(REL));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回归:从 exe 位置也要能找到句库。此前只按工作目录找,
    /// 换个方式启动 target/release 里的 exe 就是一个空句库。
    #[test]
    fn content_db_candidates_include_paths_derived_from_the_exe() {
        let exe = PathBuf::from("/repo/target/release/sentenceflow-desktop.exe");
        let got = AppPaths::content_db_candidates(None, Some(exe), None);

        // 与 exe 同级(打包安装后的样子)
        assert!(
            got.contains(&PathBuf::from("/repo/target/release/content.db")),
            "{got:?}"
        );
        // 从 target/release 退回仓库根(开发构建)
        assert!(
            got.contains(&PathBuf::from("/repo/content/build/content.db")),
            "退到仓库根的候选缺失:{got:?}"
        );
    }

    /// resource_dir 有值时必须排在最前:打包版应该用随包发的那份,
    /// 而不是碰巧躺在开发机上的仓库副本。
    #[test]
    fn resource_dir_wins_over_repo_fallbacks() {
        let got = AppPaths::content_db_candidates(
            Some(PathBuf::from("/app/resources")),
            Some(PathBuf::from("/repo/target/release/x.exe")),
            Some(PathBuf::from("/repo")),
        );
        assert_eq!(
            got.first(),
            Some(&PathBuf::from("/app/resources/content.db"))
        );
    }
}
