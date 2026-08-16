//! SentenceFlow desktop shell (Tauri 2) — spec §7.
//!
//! Architecture guarantees enforced here:
//! * 练习路径零网络调用 (§7.5): practice commands only touch sf-core + SQLite;
//!   every network request originates in workshop / ask / weekly commands.
//! * sf-core stays pure: commands are thin — load rows, call core, save rows.

mod channels;
mod commands;
mod error;
mod licensing;
mod paths;
mod progress;
mod settings;
mod state;
mod tts;
mod workshop;

use state::AppState;
use std::sync::Arc;
use tauri::Manager;

pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    tauri::Builder::default()
        .setup(|app| {
            let data_root = app
                .path()
                .app_data_dir()
                .expect("app data dir must resolve");
            let resource_dir = app.path().resource_dir().ok();
            let state = AppState::init(data_root, resource_dir)
                .map_err(|e| format!("app state init failed: {e}"))?;
            app.manage(Arc::new(state));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap,
            commands::get_settings,
            commands::set_settings,
            commands::get_license_state,
            commands::activate_license,
            commands::export_license,
            commands::list_scenes,
            commands::list_sentences,
            commands::get_sentence,
            commands::delete_user_sentence,
            commands::import_tab_sentences,
            commands::today_overview,
            commands::start_session,
            commands::start_custom_session,
            commands::submit_attempt,
            commands::judge_text,
            commands::wrongbook,
            commands::favorites,
            commands::favorite_toggle,
            commands::get_stats,
            commands::import_trial_progress,
            commands::probe_channel,
            commands::test_channel_key,
            commands::clear_channel_key,
            commands::spend_summary,
            commands::run_bench,
            commands::bench_ranking,
            commands::workshop_start,
            commands::workshop_stop,
            commands::workshop_resume,
            commands::workshop_jobs,
            commands::workshop_recover,
            commands::ask_ai,
            commands::weekly_review,
            commands::backup_export,
            commands::backup_restore,
            commands::tts_speak,
            commands::diagnostics,
        ])
        .run(tauri::generate_context!())
        .expect("error while running SentenceFlow");
}
