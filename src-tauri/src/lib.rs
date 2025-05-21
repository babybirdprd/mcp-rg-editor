mod commands;
mod config;
mod error;
mod utils;

use crate::commands::terminal_commands::ActiveSessionsMap; // For terminal sessions state
use config::init_config_state;
use std::sync::Arc;
use tauri::Manager;
use tracing::Level;
use tracing_subscriber::{filter::EnvFilter, FmtSubscriber, fmt::format::FmtSpan};


fn setup_logging(log_level_str: &str, app_handle: &tauri::AppHandle) {
    let level = match log_level_str.to_lowercase().as_str() {
        "trace" => Level::TRACE,
        "debug" => Level::DEBUG,
        "info" => Level::INFO,
        "warn" => Level::WARN,
        "error" => Level::ERROR,
        _ => Level::INFO,
    };

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("mcp_rg_editor_tauri_lib={}", level)));

    let log_plugin_builder = tauri_plugin_log::Builder::default()
        .targets([
            tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Stdout).with_level(level),
            tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                file_name: Some("app.log".into()),
            })
            .with_level(level),
            tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Webview).with_level(level),
        ])
        .level_for("hyper", Level::OFF)
        .level_for("rustls", Level::OFF)
        .with_colors(true);

    let subscriber = FmtSubscriber::builder()
        .with_env_filter(env_filter)
        .with_target(true)
        .with_ansi(false)
        .with_writer(std::io::stderr)
        .with_level(true)
        .with_span_events(FmtSpan::CLOSE)
        .finish();

    tracing::subscriber::set_global_default(subscriber)
        .expect("Failed to set tracing subscriber");

    app_handle
        .plugin(log_plugin_builder.build())
        .expect("Failed to initialize tauri-plugin-log");

    tracing::info!("Tracing subscriber initialized. Tauri-plugin-log also initialized.");
}


#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let app_handle = app.handle().clone();
            let config_state = init_config_state(&app_handle);
            app.manage(config_state.clone());

            let log_level_for_setup = config_state.read().unwrap().log_level.clone();
            setup_logging(&log_level_for_setup, &app_handle);

            let audit_logger = Arc::new(utils::audit_logger::AuditLogger::new(config_state.clone()));
            app.manage(audit_logger);

            let fuzzy_search_logger = Arc::new(utils::fuzzy_search_logger::FuzzySearchLogger::new(config_state.clone()));
            app.manage(fuzzy_search_logger);

            // Initialize ActiveSessionsMap for terminal commands
            let active_sessions_map: ActiveSessionsMap = Default::default();
            app.manage(active_sessions_map);

            // Initialize Sysinfo for process commands
            let sysinfo_state = Arc::new(tokio::sync::Mutex::new(sysinfo::System::new_all()));
            app.manage(sysinfo_state);


            if which::which("rg").is_err() {
                tracing::warn!("ripgrep (rg) is not installed or not in PATH. `search_code` tool will fail.");
                // Consider using tauri_plugin_dialog for a more integrated warning
                 tauri_plugin_notification::Notification::new(&app_handle.config().identifier)
                    .title("Ripgrep Not Found")
                    .body("The 'search_code' tool requires ripgrep (rg) to be installed and in your PATH.")
                    .icon("dialog-warning") // Requires an icon named dialog-warning or use a path
                    .show()
                    .unwrap_or_else(|e| tracing::error!("Failed to show notification: {}",e));
            }
            tracing::info!(version = %env!("CARGO_PKG_VERSION"), "MCP-RG-Editor Tauri backend setup complete.");
            Ok(())
        })
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            commands::greet,
            // Config Commands
            commands::config_commands::get_config_command,
            commands::config_commands::set_config_value_command,
            // Filesystem Commands
            commands::filesystem_commands::read_file_command,
            commands::filesystem_commands::write_file_command,
            commands::filesystem_commands::create_directory_command,
            commands::filesystem_commands::list_directory_command,
            commands::filesystem_commands::move_file_command,
            commands::filesystem_commands::get_file_info_command,
            commands::filesystem_commands::read_multiple_files_command,
            commands::filesystem_commands::search_files_command,
            // Ripgrep Commands
            commands::ripgrep_commands::search_code_command,
            // Terminal Commands
            commands::terminal_commands::execute_command,
            commands::terminal_commands::force_terminate_session_command,
            commands::terminal_commands::list_sessions_command,
            commands::terminal_commands::read_session_output_status_command,
            // Process Commands
            commands::process_commands::list_processes_command,
            commands::process_commands::kill_process_command,
            // Edit Commands
            commands::edit_commands::edit_block_command
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}