// FILE: src-tauri/src/lib.rs
// IMPORTANT NOTE: Rewrite the entire file.
mod commands;
mod config;
mod error;
mod utils;
mod mcp; // New module for MCP server logic

use crate::commands::terminal_commands::ActiveSessionsMap;
use crate::config::{Config, init_config_state, TransportMode as AppTransportMode}; // Renamed to avoid conflict
use crate::mcp::handler::EnhancedServerHandler; // MCP Handler
use crate::mcp::McpServerLaunchParams;


use std::sync::Arc;
use tauri::Manager;
use tracing::Level;
use tracing_subscriber::{filter::EnvFilter, fmt::format::FmtSpan, layer::SubscriberExt, util::SubscriberInitExt, Layer};

// MCP SDK imports
use rust_mcp_sdk::mcp_server::{server_runtime, ServerRuntime as McpServerRuntime};
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
        .with_level(true)
        .with_writer(std::io::stderr)
        .with_span_events(FmtSpan::CLOSE);

    let tauri_log_plugin_target = tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
        file_name: Some("app_ui.log".into()), // Differentiate from potential MCP server log
    })
    .with_level(level)
    .with_filter(env_filter.clone());

    let tauri_webview_target = tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::Webview)
        .with_level(Level::INFO) // Usually INFO or WARN for webview console
        .with_filter(env_filter.clone());

    tracing_subscriber::registry()
        .with(fmt_layer)
        .with(env_filter)
        .init();

    let log_plugin = tauri_plugin_log::Builder::default()
        .targets([
            tauri_log_plugin_target,
            tauri_webview_target,
        ])
        .level_for("hyper", Level::WARN)
        .level_for("rustls", Level::WARN)
        .with_colors(true)
        .build();

    app_handle
        .plugin(log_plugin)
        .expect("Failed to initialize tauri-plugin-log for UI");

    tracing::info!("Tracing subscriber and tauri-plugin-log for UI initialized. Log level: {}", level);
}

fn get_mcp_server_details(app_config: &Config) -> McpInitializeResult {
    // Use app_config if needed to customize server_info, or use hardcoded values
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

async fn map_mcp_sdk_error_async(err: McpSdkError) -> anyhow::Error {
    anyhow::anyhow!("MCP SDK Error: {:?}", err)
}


#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let app_handle = app.handle().clone();
            let config_state_arc = init_config_state(&app_handle); // Arc<RwLock<Config>>

            // Setup UI logging first
            let log_level_for_setup = config_state_arc.read().unwrap().log_level.clone();
            setup_tracing_and_logging(&log_level_for_setup, &app_handle);

            app.manage(config_state_arc.clone()); // Manage for Tauri commands

            let audit_logger = Arc::new(utils::audit_logger::AuditLogger::new(config_state_arc.clone()));
            app.manage(audit_logger);

            let fuzzy_search_logger = Arc::new(utils::fuzzy_search_logger::FuzzySearchLogger::new(config_state_arc.clone()));
            app.manage(fuzzy_search_logger);

            let active_sessions_map: ActiveSessionsMap = Default::default();
            app.manage(active_sessions_map);

            let sysinfo_state = Arc::new(tokio::sync::Mutex::new(sysinfo::System::new_all()));
            app.manage(sysinfo_state);

            // Prepare MCP Server Launch Params
            // The AppHandle needs to be captured for the async block
            let mcp_app_handle_clone = app_handle.clone();
            let mcp_config_state_clone = config_state_arc.clone(); // Clone Arc for the async block

            let mcp_launch_params = McpServerLaunchParams {
                app_handle: mcp_app_handle_clone,
                config_state: mcp_config_state_clone,
            };
            
            // Spawn the MCP server in a separate Tokio task
            tokio::spawn(async move {
                tracing::info!("Attempting to start MCP server...");
                // Determine MCP transport mode from config
                let transport_mode_from_config = { // Read config for transport mode
                    let cfg_guard = mcp_launch_params.config_state.read().expect("Failed to read config for MCP transport");
                    // Assuming your AppConfig has a field like `mcp_transport_mode: String` ("stdio" or "sse")
                    // For now, let's default to stdio if not specified, or use a specific feature flag.
                    // We'll use the feature flags `mcp-stdio-server` and `mcp-sse-server`
                    #[cfg(feature = "mcp-sse-server")]
                    let mode = AppTransportMode::Sse;
                    #[cfg(not(feature = "mcp-sse-server"))]
                    #[cfg(feature = "mcp-stdio-server")]
                    let mode = AppTransportMode::Stdio;
                    #[cfg(not(any(feature = "mcp-sse-server", feature = "mcp-stdio-server")))]
                    let mode = {
                        tracing::warn!("No MCP server transport feature enabled, defaulting to STDIO for MCP server.");
                        AppTransportMode::Stdio // Fallback or could error out
                    };
                    mode
                };

                let mcp_server_details = {
                    let cfg_guard = mcp_launch_params.config_state.read().expect("Failed to read config for MCP details");
                    get_mcp_server_details(&cfg_guard)
                };

                // Create EnhancedServerHandler, passing necessary states/handles
                // The handler will need access to AppHandle for plugins and Config for its operations
                let mcp_handler = EnhancedServerHandler::new(mcp_launch_params.app_handle.clone(), mcp_launch_params.config_state.clone());

                match transport_mode_from_config {
                    #[cfg(feature = "mcp-stdio-server")]
                    AppTransportMode::Stdio => {
                        tracing::info!("Starting MCP server with STDIO transport.");
                        let mcp_transport_opts = McpTransportOptions::default();
                        match McpStdioTransport::new(mcp_transport_opts) {
                            Ok(transport) => {
                                let mcp_server_runtime: McpServerRuntime<_> = server_runtime::create_server(mcp_server_details, transport, mcp_handler);
                                if let Err(e) = mcp_server_runtime.start().await.map_err(map_mcp_sdk_error_async).await {
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
                            // These would come from your Config struct, e.g. cfg_guard.mcp_sse_host, cfg_guard.mcp_sse_port
                            // For now, using placeholders or env vars directly if Config isn't fully adapted yet.
                            let sse_host = std::env::var("MCP_SSE_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
                            let sse_port = std::env::var("MCP_SSE_PORT").unwrap_or_else(|_| "3030".to_string()).parse::<u16>().unwrap_or(3030);
                            (sse_host, sse_port)
                        };
                        tracing::info!("Starting MCP server with SSE transport on {}:{}", host, port);
                        let mcp_sse_options = McpHyperServerOptions {
                            host,
                            port,
                            enable_cors: true,
                            ..Default::default()
                        };
                        let mcp_sse_server_runtime: McpHyperServerRuntime<_> = create_mcp_sse_server(mcp_server_details, mcp_handler, mcp_sse_options);
                         if let Err(e) = mcp_sse_server_runtime.start().await.map_err(map_mcp_sdk_error_async).await {
                            tracing::error!("MCP SSE Server failed to start or shut down with error: {:?}", e);
                        } else {
                            tracing::info!("MCP SSE Server shut down.");
                        }
                    }
                    _ => { // Handles Tauri mode or if features are mismatched
                        tracing::info!("MCP server not started for current AppTransportMode or feature set.");
                    }
                }
            });


            if which::which("rg").is_err() {
                tracing::warn!("ripgrep (rg) is not installed or not in PATH. `search_code` tool will fail.");
                let _ = tauri_plugin_dialog::Builder::new(app_handle.clone())
                    .title("Ripgrep Not Found")
                    .message("The 'search_code' tool requires ripgrep (rg) to be installed and in your system's PATH for full functionality.")
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
        // tauri-plugin-log is initialized in setup_tracing_and_logging
        .invoke_handler(tauri::generate_handler![
            commands::greet,
            // Config Commands (for UI interaction with the shared Config)
            commands::config_commands::get_config_command,
            commands::config_commands::set_config_value_command,
            // UI specific commands that might wrap/call tool logic
            // For now, we assume the MCP server handles tool calls.
            // If the UI needs to directly trigger tool actions without going through MCP network layer,
            // we'd add more Tauri commands here that call the underlying tool functions.
            // Example: commands::filesystem_commands::ui_read_file_command,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}