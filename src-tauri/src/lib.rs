// FILE: src-tauri/src/lib.rs
mod commands;
mod config;
mod error;
mod utils;
mod mcp;

use crate::commands::terminal_commands::ActiveSessionsMap;
use crate::config::{Config, init_config_state, TransportMode as AppTransportMode};
use crate::mcp::handler::EnhancedServerHandler;
use crate::mcp::McpServerLaunchParams;
use crate::commands::process_commands::SysinfoState;

use std::sync::Arc;
use tauri::{Manager, plugin::Plugin}; // Added Plugin trait for log plugin
use tracing::Level;
use tracing_subscriber::{filter::EnvFilter, fmt::format::FmtSpan, layer::SubscriberExt, util::SubscriberInitExt, Layer};

use rust_mcp_sdk::mcp_server::{server_runtime, ServerRuntime as McpServerRuntime, McpServer}; // Added McpServer trait
use rust_mcp_sdk::error::McpSdkError;
use rust_mcp_schema::{InitializeResult as McpInitializeResult, Implementation as McpImplementation, ServerCapabilities as McpServerCapabilities, ServerCapabilitiesTools as McpServerCapabilitiesTools, LATEST_PROTOCOL_VERSION as MCP_LATEST_PROTOCOL_VERSION};
use rust_mcp_transport::{StdioTransport as McpStdioTransport, TransportOptions as McpTransportOptions};

#[cfg(feature = "mcp-sse-server")]
use rust_mcp_sdk::hyper_server::{create_server as create_mcp_sse_server, HyperServerOptions as McpHyperServerOptions, HyperServerRuntime as McpHyperServerRuntime};


fn setup_tracing_and_logging(log_level_str: &str, app_handle: &tauri::AppHandle) {
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

    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_ansi(false)
        .with_writer(std::io::stderr)
        .with_level(true)
        .with_span_events(FmtSpan::CLOSE);

    let tauri_log_plugin_target_logdir = tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
        file_name: Some("app_ui.log".into()),
    })
    .level(level)
    .filter(env_filter.clone());

    let tauri_webview_target = tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Webview)
        .level(Level::INFO)
        .filter(env_filter.clone());

    tracing_subscriber::registry()
        .with(fmt_layer)
        .with(env_filter)
        .init();

    let log_plugin_builder = tauri_plugin_log::Builder::default()
        .targets([
            tauri_log_plugin_target_logdir,
            tauri_webview_target,
        ])
        .level_for("hyper", log::LevelFilter::Warn)
        .level_for("rustls", log::LevelFilter::Warn);
        // .colors(true) // Check if tauri_plugin_log::Builder has .colors() or similar

    // It seems tauri_plugin_log::Builder does not have a .build() that returns impl Plugin
    // Instead, it's directly used in app.plugin()
    app_handle
        .plugin(log_plugin_builder.build()) // Assuming build() returns the plugin
        .expect("Failed to initialize tauri-plugin-log for UI");

    tracing::info!("Tracing subscriber and tauri-plugin-log for UI initialized. Log level: {}", level);
}

fn get_mcp_server_details(_app_config: &Config) -> McpInitializeResult {
    McpInitializeResult {
        server_info: McpImplementation {
            name: "mcp-rg-editor-tauri-hosted".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
        capabilities: McpServerCapabilities {
            tools: Some(McpServerCapabilitiesTools { list_changed: None }),
            resources: Some(Default::default()),
            prompts: Some(Default::default()),
            ..Default::default()
        },
        meta: None,
        instructions: Some(
            "MCP Server hosted within Tauri. Tools interact with local system via Tauri plugins.".to_string()
        ),
        protocol_version: MCP_LATEST_PROTOCOL_VERSION.to_string(),
    }
}

fn map_mcp_sdk_error_sync(err: McpSdkError) -> anyhow::Error {
    anyhow::anyhow!("MCP SDK Error: {:?}", err)
}


#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let app_handle = app.handle().clone();
            let config_state_arc = init_config_state(&app_handle);

            let log_level_for_setup = config_state_arc.read().unwrap().log_level.clone();
            setup_tracing_and_logging(&log_level_for_setup, &app_handle);

            app.manage(config_state_arc.clone());

            let audit_logger = Arc::new(utils::audit_logger::AuditLogger::new(config_state_arc.clone()));
            app.manage(audit_logger);

            let fuzzy_search_logger = Arc::new(utils::fuzzy_search_logger::FuzzySearchLogger::new(config_state_arc.clone()));
            app.manage(fuzzy_search_logger);

            let active_sessions_map: ActiveSessionsMap = Default::default();
            app.manage(active_sessions_map);

            let sysinfo_state: SysinfoState = Arc::new(tokio::sync::Mutex::new(sysinfo::System::new_all()));
            app.manage(sysinfo_state);

            let mcp_app_handle_clone = app_handle.clone();
            let mcp_config_state_clone = config_state_arc.clone();

            let mcp_launch_params = McpServerLaunchParams {
                app_handle: mcp_app_handle_clone,
                config_state: mcp_config_state_clone,
            };
            
            tokio::spawn(async move {
                tracing::info!("Attempting to start MCP server...");
                let transport_mode_from_config = {
                    let cfg_guard = mcp_launch_params.config_state.read().expect("Failed to read config for MCP transport");
                    cfg_guard.mcp_transport_mode.clone()
                };

                let mcp_server_details = {
                    let cfg_guard = mcp_launch_params.config_state.read().expect("Failed to read config for MCP details");
                    get_mcp_server_details(&cfg_guard)
                };

                let mcp_handler = EnhancedServerHandler::new(mcp_launch_params.app_handle.clone(), mcp_launch_params.config_state.clone());

                match transport_mode_from_config {
                    #[cfg(feature = "mcp-stdio-server")]
                    AppTransportMode::Stdio => {
                        tracing::info!("Starting MCP server with STDIO transport.");
                        let mcp_transport_opts = McpTransportOptions::default();
                        match McpStdioTransport::new(mcp_transport_opts) {
                            Ok(transport) => {
                                let mcp_server_runtime: McpServerRuntime<_,_> = server_runtime::create_server(mcp_server_details, transport, mcp_handler);
                                if let Err(e) = mcp_server_runtime.start().await.map_err(map_mcp_sdk_error_sync) {
                                    tracing::error!("MCP STDIO Server failed to start or shut down with error: {:?}", e);
                                } else {
                                    tracing::info!("MCP STDIO Server shut down.");
                                }
                            }
                            Err(e) => {
                                tracing::error!("Failed to create MCP StdioTransport: {}", e);
                            }
                        }
                    }
                    #[cfg(feature = "mcp-sse-server")]
                    AppTransportMode::Sse => {
                        let (host, port) = {
                            let cfg_guard = mcp_launch_params.config_state.read().expect("Failed to read config for SSE params");
                            let sse_host = cfg_guard.mcp_sse_host.clone().unwrap_or_else(|| "127.0.0.1".to_string());
                            let sse_port = cfg_guard.mcp_sse_port.unwrap_or(3030); // Default SSE port
                            (sse_host, sse_port)
                        };
                        tracing::info!("Starting MCP server with SSE transport on {}:{}", host, port);
                        let mcp_sse_options = McpHyperServerOptions {
                            host,
                            port,
                            enable_cors: true,
                            ..Default::default()
                        };
                        let mcp_sse_server_runtime: McpHyperServerRuntime<_,_> = create_mcp_sse_server(mcp_server_details, mcp_handler, mcp_sse_options);
                         if let Err(e) = mcp_sse_server_runtime.start().await.map_err(map_mcp_sdk_error_sync) {
                            tracing::error!("MCP SSE Server failed to start or shut down with error: {:?}", e);
                        } else {
                            tracing::info!("MCP SSE Server shut down.");
                        }
                    }
                     _ => { // This handles cases where the feature for the configured mode is not active.
                        if transport_mode_from_config == AppTransportMode::Stdio && !cfg!(feature="mcp-stdio-server") {
                             tracing::error!("MCP_TRANSPORT is 'stdio' but 'mcp-stdio-server' feature is not enabled in Cargo.toml.");
                        } else if transport_mode_from_config == AppTransportMode::Sse && !cfg!(feature="mcp-sse-server") {
                             tracing::error!("MCP_TRANSPORT is 'sse' but 'mcp-sse-server' feature is not enabled in Cargo.toml.");
                        } else if transport_mode_from_config != AppTransportMode::Stdio && transport_mode_from_config != AppTransportMode::Sse {
                             tracing::warn!("Unknown MCP_TRANSPORT mode configured: {:?}. MCP server not started.", transport_mode_from_config);
                        } else {
                             // This case implies the mode is valid but the corresponding feature is off.
                             // The specific error messages above cover this.
                             tracing::info!("MCP server not started as the configured transport mode ({:?}) feature is not enabled.", transport_mode_from_config);
                        }
                    }
                }
            });


            if which::which("rg").is_err() {
                tracing::warn!("ripgrep (rg) is not installed or not in PATH. `search_code` tool will fail.");
                let dialog_handle = app_handle.dialog(); // Use DialogExt
                dialog_handle
                    .message("The 'search_code' tool requires ripgrep (rg) to be installed and in your system's PATH for full functionality.")
                    .title("Ripgrep Not Found")
                    .kind(tauri_plugin_dialog::MessageDialogKind::Warning)
                    .ok_button_label("OK")
                    .show(|_| {});
            }
            tracing::info!(version = %env!("CARGO_PKG_VERSION"), "MCP-RG-Editor Tauri UI backend setup complete.");
            Ok(())
        })
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        // Note: tauri-plugin-log is initialized in setup_tracing_and_logging
        .invoke_handler(tauri::generate_handler![
            commands::greet,
            commands::config_commands::get_config_command,
            commands::config_commands::set_config_value_command,
            commands::filesystem_commands::read_file_command,
            commands::filesystem_commands::read_multiple_files_command,
            commands::filesystem_commands::write_file_command,
            commands::filesystem_commands::create_directory_command,
            commands::filesystem_commands::list_directory_command,
            commands::filesystem_commands::move_file_command,
            commands::filesystem_commands::get_file_info_command,
            commands::filesystem_commands::search_files_command,
            commands::ripgrep_commands::search_code_command,
            // Terminal commands are primarily for MCP, UI might not need direct invoke
            // commands::terminal_commands::execute_command_ui, 
            // commands::terminal_commands::force_terminate_session_ui,
            // commands::terminal_commands::list_sessions_ui,
            // commands::terminal_commands::read_session_output_status_ui,
            commands::process_commands::list_processes_command,
            commands::process_commands::kill_process_command,
            commands::edit_commands::edit_block_command,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}