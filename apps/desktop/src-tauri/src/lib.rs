//! SentenceFlow desktop shell (Tauri 2) — spec §7.
//!
//! Architecture guarantees enforced here:
//! * 练习路径零网络调用 (§7.5): practice commands only touch sf-core + SQLite;
//!   every network request originates in workshop / ask / weekly commands.
//! * sf-core stays pure: commands are thin — load rows, call core, save rows.

mod channels;
mod chat;
mod commands;
mod error;
mod installer;
mod licensing;
mod paths;
mod pet;
mod progress;
mod settings;
mod skills;
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
        // AI 萌宠:提醒走系统通知,咒语包里的平台链接走系统浏览器。
        // 两者都只由 Rust 侧调用/受 capability 白名单约束,不放宽 CSP。
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_root = app
                .path()
                .app_data_dir()
                .expect("app data dir must resolve");
            let resource_dir = app.path().resource_dir().ok();
            let state = AppState::init(data_root, resource_dir)
                .map_err(|e| format!("app state init failed: {e}"))?;
            app.manage(Arc::new(state));
            // 仅当 settings.pet.enabled(默认 false)才建宠物窗与心跳线程;
            // 默认关闭时这里是纯空转,行为与整合前逐项一致。
            pet::setup(app.handle());
            Ok(())
        })
        // 句流原本没有窗口事件处理器,这里是纯新增:只认宠物窗,
        // 主窗关闭 = 退出进程的既有语义不受影响。
        .on_window_event(pet::on_window_event)
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
            commands::delete_user_sentences_by_scene,
            commands::import_tab_sentences,
            commands::today_overview,
            commands::start_session,
            commands::start_custom_session,
            commands::submit_attempt,
            commands::judge_text,
            commands::placement_start,
            commands::placement_answer,
            commands::placement_last,
            commands::list_scene_packs,
            commands::list_pack_sentences,
            commands::start_scenario_session,
            commands::delete_user_scene_pack,
            commands::delete_user_scene_packs,
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
            chat::chat_thread_create,
            chat::chat_threads,
            chat::chat_history,
            chat::chat_thread_delete,
            chat::chat_thread_set_model,
            chat::chat_send,
            chat::chat_stop,
            chat::chat_active_threads,
            chat::agent_send,
            chat::pick_folder,
            skills::skill_catalog,
            skills::skill_source,
            skills::skill_save,
            skills::skill_delete,
            commands::pick_file,
            commands::backup_export,
            commands::backup_restore,
            commands::tts_speak,
            commands::diagnostics,
            commands::export_diagnostics,
            // ---- AI 萌宠(54 个 pet_*;§6.1 命令总表 + open_url/reveal_in_dir)----
            pet::commands::pet_bootstrap,
            pet::commands::pet_settings_get,
            pet::commands::pet_settings_set,
            pet::commands::pet_list,
            pet::commands::pet_thumb,
            pet::commands::pet_active_assets,
            pet::commands::pet_set_active,
            pet::commands::pet_set_anchors,
            pet::commands::pet_delete,
            pet::commands::pet_rig_bake,
            pet::commands::pet_kit_export,
            pet::commands::pet_kit_import,
            pet::commands::pet_kit_validate,
            pet::commands::pet_wizard_reset,
            pet::commands::pet_wizard_session,
            pet::commands::pet_wizard_add_strip,
            pet::commands::pet_wizard_add_sheet,
            pet::commands::pet_wizard_add_video,
            pet::commands::pet_wizard_add_video_multi,
            pet::commands::pet_wizard_swap_states,
            pet::commands::pet_wizard_evolve,
            pet::commands::pet_wizard_remove_strip,
            pet::commands::pet_wizard_preview,
            pet::commands::pet_wizard_hatch,
            pet::commands::pet_wizard_reslice,
            pet::commands::pet_wizard_replace_frame,
            pet::commands::pet_wizard_export_frame,
            pet::commands::pet_l0_preview,
            pet::commands::pet_l0_hatch,
            pet::commands::pet_reference_export,
            pet::commands::pet_watcher_start,
            pet::commands::pet_watcher_stop,
            pet::commands::pet_spellbook_get,
            pet::commands::pet_guide_save,
            pet::commands::pet_rescue_get,
            pet::commands::pet_reminders_list,
            pet::commands::pet_reminder_create,
            pet::commands::pet_reminder_toggle,
            pet::commands::pet_reminder_delete,
            pet::commands::pet_reminder_snooze,
            pet::commands::pet_pomodoro_start,
            pet::commands::pet_pomodoro_stop,
            pet::commands::pet_pomodoro_status,
            pet::commands::pet_exporter_status,
            pet::commands::pet_export_birth_video,
            pet::commands::pet_work_area_get,
            pet::commands::pet_cursor_position_get,
            pet::commands::pet_visible_set,
            pet::commands::pet_click_through_set,
            pet::commands::pet_open_studio,
            pet::commands::pet_pick_file,
            pet::commands::pet_save_path,
            pet::commands::pet_open_url,
            pet::commands::pet_reveal_in_dir,
        ])
        .build(tauri::generate_context!())
        .expect("error while building SentenceFlow")
        .run(|app, event| {
            // 退出前收干净 AI 萌宠的后台执行体(心跳线程、下载监听);
            // 主窗关闭 = 退出进程的语义不变。
            if let tauri::RunEvent::Exit = event {
                pet::shutdown(app);
            }
        });
}
