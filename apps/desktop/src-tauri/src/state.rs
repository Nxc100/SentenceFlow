//! Application state shared across Tauri commands.
//!
//! Locking order (always acquire in this order, never hold across await):
//! `progress` → `content` → `settings`. The workshop runner owns its own
//! state and talks to the DBs through short-lived locks.

use crate::error::{CmdError, CmdResult};
use crate::paths::AppPaths;
use crate::progress::ProgressDb;
use crate::settings::{SETTINGS_KEY, Settings};
use sf_core::sentence::LevelId;
use sf_core::spec::LevelSpec;
use sf_pipeline::lexicon::Lexicon;
use sf_pipeline::store::{ContentIndex, ContentStore};
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

pub struct AppState {
    pub paths: AppPaths,
    pub progress: Mutex<ProgressDb>,
    pub content: Mutex<ContentIndex>,
    pub specs: BTreeMap<LevelId, LevelSpec>,
    pub lexicon: Lexicon,
    pub settings: Mutex<Settings>,
    pub policy: sf_llm::policy::ChannelPolicy,
    /// Piper sidecar when the audio pack is installed; None = frontend falls
    /// back to the WebView's speechSynthesis.
    pub tts: Option<crate::tts::PiperTts>,
    /// Set while a workshop job is streaming; [停止] flips it.
    pub gen_cancel: AtomicBool,
}

impl AppState {
    pub fn init(
        data_root: std::path::PathBuf,
        resource_dir: Option<std::path::PathBuf>,
    ) -> CmdResult<Self> {
        let paths = AppPaths::new(data_root)?;
        let progress = ProgressDb::open(&paths.progress_db())?;
        let tts = crate::tts::PiperTts::detect(resource_dir.clone(), paths.root.join("tts-cache"));
        let bundled_channels_json = resource_dir.as_ref().map(|r| r.join("channels.json"));

        let content_path = AppPaths::bundled_content_db(resource_dir).ok_or_else(|| {
            CmdError::new(
                "content",
                "content.db not found — run `cargo run -p sf-pipeline --features factory --bin sf -- factory build` first",
            )
        })?;
        let factory = ContentStore::open_readonly(&content_path)?;
        let user = ContentStore::open_rw(&paths.user_content_db()).ok();

        // Spec snapshot ships inside content.db (内容与行为同版, §7.7).
        let specs: BTreeMap<LevelId, LevelSpec> = factory
            .load_level_specs()?
            .into_iter()
            .map(|s| (s.id, s))
            .collect();
        if specs.is_empty() {
            return Err(CmdError::new(
                "content",
                "content.db carries no level specs",
            ));
        }

        // Lemma table doubles as the client lexicon (validation + dictionary).
        let lexicon =
            Lexicon::from_tsv(&factory.lemma_tsv()?).map_err(|e| CmdError::new("content", e))?;

        let settings: Settings = progress
            .kv_get(SETTINGS_KEY)?
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();

        // channels.json 三级回退(§3.6):app-data 副本(内容包热更新)→
        // 随包资源 → 内置默认。此前漏掉了随包资源一级,策略更新只能靠
        // 热更新才生效 —— 现在装包即带最新策略。
        let policy = [Some(paths.channels_json()), bundled_channels_json]
            .into_iter()
            .flatten()
            .find_map(|p| {
                std::fs::read_to_string(p)
                    .ok()
                    .and_then(|raw| sf_llm::policy::ChannelPolicy::from_json(&raw).ok())
            })
            .unwrap_or_default();

        Ok(Self {
            paths,
            progress: Mutex::new(progress),
            content: Mutex::new(ContentIndex { factory, user }),
            specs,
            lexicon,
            settings: Mutex::new(settings),
            policy,
            tts,
            gen_cancel: AtomicBool::new(false),
        })
    }

    pub fn spec_for(&self, level: LevelId) -> CmdResult<&LevelSpec> {
        self.specs
            .get(&level)
            .ok_or_else(|| CmdError::new("content", format!("no spec for {level}")))
    }

    pub fn save_settings(&self, settings: &Settings) -> CmdResult<()> {
        let progress = self.progress.lock().expect("progress lock");
        progress.kv_set(SETTINGS_KEY, &serde_json::to_string(settings)?)?;
        Ok(())
    }

    pub fn request_gen_cancel(&self) {
        self.gen_cancel.store(true, Ordering::SeqCst);
    }

    pub fn take_gen_cancel(&self) -> bool {
        self.gen_cancel.swap(false, Ordering::SeqCst)
    }

    pub fn gen_cancelled(&self) -> bool {
        self.gen_cancel.load(Ordering::SeqCst)
    }
}

pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// Timezone note: day-boundary math needs the user's UTC offset; the frontend
// passes `-new Date().getTimezoneOffset() * 60` into the stats command rather
// than the backend guessing it.
