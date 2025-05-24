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

use std::sync::Arc;
use tauri::Manager;
use tracing::Level;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};

use rust_mcp_sdk::McpServer;
use rust_mcp_sdk::mcp_server::{server_runtime, ServerRuntime as McpServerRuntime};
use rust_mcp_sdk::error::McpSdkError; // Make sure McpSdkError is in scope
use rust_mcp_schema::{InitializeResult as McpInitializeResult, Implementation as McpImplementation, ServerCapabilities as McpServerCapabilities, ServerCapabilitiesTools as McpServerCapabilitiesTools, LATEST_PROTOCOL_VERSION as MCP_LATEST_PROTOCOL_VERSION};
use rust_mcp_transport::{StdioTransport as McpStdioTransport, TransportOptions as McpTransportOptions};


#[cfg(feature = "mcp-sse-server")]
use rust_mcp_sdk::mcp_server::{
    hyper_server::create_server as create_mcp_sse_server,
    HyperServerOptions as McpHyperServerOptions,
    HyperServer as McpHyperServerRuntime // This is an alias for the HyperServer struct from the SDK
    // Note: If McpHyperServerRuntime was intended to be a different type, this alias might need adjustment
    // based on what `rust-mcp-sdk::mcp_server` actually exports as `HyperServerRuntime`.
    // Given the SDK structure, `HyperServer` is the struct that `create_server` returns and that has the `start` method.
};

fn setup_tracing_and_logging(log_level_str: &str, app_handle: &tauri::AppHandle) {
    let level = match log_level_str.to_lowercase().as_str() {
        "trace" => Level::TRACE,
        "debug" => Level::DEBUG,
        "info" => Level::INFO,
        "warn" => Level::WARN,
        "error" => Level::ERROR,
        _ => Level::INFO,
    };

    let tauri_log_targets = [
        tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
            file_name: Some("app_backend.log".into()),
        }),
        tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Webview),
    ];

    let log_plugin_instance = tauri_plugin_log::Builder::default()
        .targets(tauri_log_targets)
        .level_for("hyper", log::LevelFilter::Warn)
        .level_for("rustls", log::LevelFilter::Warn)
        .level(match level {
            Level::TRACE => log::LevelFilter::Trace,
            Level::DEBUG => log::LevelFilter::Debug,
            Level::INFO => log::LevelFilter::Info,
            Level::WARN => log::LevelFilter::Warn,
            Level::ERROR => log::LevelFilter::Error,
        })
        .build();

    app_handle.plugin(log_plugin_instance).expect("Failed to initialize tauri-plugin-log");
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

            let sysinfo_state_for_mcp_and_commands = Arc::new(tokio::sync::Mutex::new(sysinfo::System::new_all()));
            app.manage(sysinfo_state_for_mcp_and_commands.clone());


            let mcp_app_handle_clone = app_handle.clone();
            let mcp_config_state_clone = config_state_arc.clone();

            let mcp_launch_params = McpServerLaunchParams {
                app_handle: mcp_app_handle_clone,
                config_state: mcp_config_state_clone,
            };

            tauri::async_runtime::spawn(async move {
                tracing::info!("Attempting to start MCP server...");
                let transport_mode_from_config = {
                    let cfg_guard = mcp_launch_params.config_state.read().expect("Failed to read config for MCP transport");
                    cfg_guard.mcp_transport_mode.clone()
                };

                if transport_mode_from_config == AppTransportMode::Disabled {
                    tracing::info!("MCP_TRANSPORT is 'disabled'. MCP server will not be started.");
                    return;
                }

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
                                let mcp_server_runtime: McpServerRuntime = server_runtime::create_server(mcp_server_details, transport, mcp_handler);
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
                            let sse_port = cfg_guard.mcp_sse_port.unwrap_or(3030);
                            (sse_host, sse_port)
                        };
                        tracing::info!("Starting MCP server with SSE transport on {}:{}", host, port);
                        let mcp_sse_options = McpHyperServerOptions {
                            host,
                            port,
                            // enable_cors: true, // REMOVED: This field does not exist in rust-mcp-sdk 0.2.6 HyperServerOptions
                            ..Default::default()
                        };
                        let mcp_sse_server_runtime: McpHyperServerRuntime = create_mcp_sse_server(mcp_server_details, mcp_handler, mcp_sse_options);
                         if let Err(e) = mcp_sse_server_runtime.start().await
                            .map_err(|transport_server_err| { // transport_server_err is TransportServerError
                                let mcp_sdk_err: McpSdkError = transport_server_err.into(); // Convert to McpSdkError
                                map_mcp_sdk_error_sync(mcp_sdk_err) // Now this matches the function signature
                            }) {
                            tracing::error!("MCP SSE Server failed to start or shut down with error: {:?}", e);
                        } else {
                            tracing::info!("MCP SSE Server shut down.");
                        }
                    }
                     _ => {
                        if transport_mode_from_config == AppTransportMode::Stdio && !cfg!(feature="mcp-stdio-server") {
                             tracing::error!("MCP_TRANSPORT is 'stdio' but 'mcp-stdio-server' feature is not enabled in Cargo.toml.");
                        } else if transport_mode_from_config == AppTransportMode::Sse && !cfg!(feature="mcp-sse-server") {
                             tracing::error!("MCP_TRANSPORT is 'sse' but 'mcp-sse-server' feature is not enabled in Cargo.toml.");
                        } else if transport_mode_from_config != AppTransportMode::Stdio && transport_mode_from_config != AppTransportMode::Sse && transport_mode_from_config != AppTransportMode::Disabled {
                             tracing::warn!("Unknown MCP_TRANSPORT mode configured: {:?}. MCP server not started.", transport_mode_from_config);
                        } else if transport_mode_from_config != AppTransportMode::Disabled {
                             tracing::info!("MCP server not started as the configured transport mode ({:?}) feature is not enabled.", transport_mode_from_config);
                        }
                    }
                }
            });


            if which::which("rg").is_err() {
                tracing::warn!("ripgrep (rg) is not installed or not in PATH. `search_code` tool will fail.");
                let dialog_handle = app_handle.dialog();
                dialog_handle
                    .message("The 'search_code' tool requires ripgrep (rg) to be installed and in your system's PATH for full functionality.")
                    .title("Ripgrep Not Found")
                    .kind(tauri_plugin_dialog::MessageDialogKind::Warning)
                    .buttons(MessageDialogButtons::Ok)
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
        .invoke_handler(tauri::generate_handler![
            commands::greet,
            commands::config_commands::get_config_command,
            commands::config_commands::set_config_value_command,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}