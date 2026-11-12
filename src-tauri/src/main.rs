// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod oauth;
mod plugin;
mod process;
mod proxy;
mod quota;
mod responses_websocket;
mod terminal;
mod tray;

use commands::*;
use tauri::Manager;

fn main() {
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == "--install-codex-plugin")
    {
        match tauri::async_runtime::block_on(plugin::install_codex_plugin()) {
            Ok(message) => println!("{message}"),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(1);
            }
        }
        return;
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .setup(|app| {
            crate::commands::sync_account_pools(&crate::commands::get_codex_home());
            restore_proxy_on_startup();
            #[cfg(windows)]
            tauri::async_runtime::spawn(async {
                match plugin::install_codex_plugin().await {
                    Ok(message) => terminal::log_terminal("INFO", &message),
                    Err(error) => terminal::log_terminal("WARN", &error),
                }
            });
            start_background_quota_refresher();

            // Setup system tray
            if let Err(e) = tray::setup_system_tray(app.handle()) {
                eprintln!("[Codex Tool] Error setting up system tray: {}", e);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_engine_status,
            set_engine_paused,
            list_thread_goals,
            reactivate_goal,
            toggle_auto_resume,
            get_config_bind_status,
            reset_codex_integration,
            enable_codex_integration,
            export_backup,
            import_backup,
            open_backups_folder,
            list_accounts,
            switch_account,
            remove_account,
            add_account_login,
            refresh_accounts_quota,
            get_terminal_logs,
            clear_terminal_logs,
            plugin::continuation_message
        ])
        .run(tauri::generate_context!())
        .expect("error while running Codex Tool application");
}
