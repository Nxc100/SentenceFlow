//! SentenceFlow desktop shell (Tauri 2) — spec §7.
//!
//! Architecture guarantees enforced here:
//! * 练习路径零网络调用 (§7.5): practice commands only touch sf-core + SQLite;
//!   every network request originates in workshop / ask / weekly commands.
//! * sf-core stays pure: commands are thin — load rows, call core, save rows.

mod channels;
mod commands;
mod error;
mod installer;
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

/// 子进程继承本进程的错误模式:损坏/系统不兼容的 CLI(如老 Win10 上的
/// opencode)不再弹系统级"无法定位程序输入点"对话框,改由命令层返回
/// 可读的中文错误(真机踩坑,见 installer.rs)。
#[cfg(windows)]
fn suppress_child_error_dialogs() {
    const SEM_FAILCRITICALERRORS: u32 = 0x0001;
    const SEM_NOOPENFILEERRORBOX: u32 = 0x8000;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn SetErrorMode(mode: u32) -> u32;
    }
    unsafe {
        SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOOPENFILEERRORBOX);
    }
}

pub fn run() {
    #[cfg(windows)]
    suppress_child_error_dialogs();
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
            commands::placement_start,
            commands::placement_answer,
            commands::placement_last,
            commands::wrongbook,
            commands::favorites,
            commands::favorite_toggle,
            commands::get_stats,
            commands::import_trial_progress,
            commands::probe_channel,
            commands::opencode_install,
            commands::opencode_login,
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
