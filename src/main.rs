mod config;
mod error;
mod mcp;
mod tools;
mod utils;

use crate::config::{Config, TransportMode};
use crate::mcp::handler::EnhancedServerHandler;
use anyhow::Result;
use rust_mcp_sdk::mcp_server::{server_runtime, McpServer};
use rust_mcp_schema::{InitializeResult, Implementation, ServerCapabilities, ServerCapabilitiesTools, LATEST_PROTOCOL_VERSION};
use rust_mcp_transport::{StdioTransport, TransportOptions};
use std::sync::Arc;
use tracing::Level;
use tracing_subscriber::{filter::EnvFilter, FmtSubscriber};

#[cfg(feature = "sse")]
use rust_mcp_transport_sse::{HyperServerOptions, server_runtime_sse};


fn setup_logging(log_level_str: &str) {
    let level = match log_level_str.to_lowercase().as_str() {
        "trace" => Level::TRACE,
        "debug" => Level::DEBUG,
        "info" => Level::INFO,
        "warn" => Level::WARN,
        "error" => Level::ERROR,
        _ => Level::INFO,
    };

    let env_filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("mcp_rg_enhanced={}", level)));

    let subscriber = FmtSubscriber::builder()
        .with_env_filter(env_filter)
        .with_target(true) // Include target for better context
        .with_ansi(false) // Disable ANSI for cleaner logs, especially over MCP
        .with_writer(std::io::stderr) // Log to stderr
        .with_level(true) // Include log level in output
        .json() // Output logs in JSON format for better machine readability if needed
        .finish();

    tracing::subscriber::set_global_default(subscriber)
        .expect("Failed to set tracing subscriber");
}

fn get_server_details() -> InitializeResult {
    InitializeResult {
        server_info: Implementation {
            name: "mcp-rg-editor".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
        capabilities: ServerCapabilities {
            tools: Some(ServerCapabilitiesTools { list_changed: None }),
            ..Default::default()
        },
        meta: None,
        instructions: Some(
            "Enhanced MCP Server for Ripgrep, Filesystem, Terminal, and Process operations. \
            All paths should be absolute or relative to FILES_ROOT. \
            Use `get_config` to see current FILES_ROOT and other settings.".to_string()
        ),
        protocol_version: LATEST_PROTOCOL_VERSION.to_string(),
    }
}


#[tokio::main]
async fn main() -> Result<()> {
    let config = Arc::new(Config::load().expect("Failed to load configuration."));
    setup_logging(&config.log_level);

    tracing::info!(version = %env!("CARGO_PKG_VERSION"), "Starting mcp-rg-editor server");
    tracing::debug!("Loaded configuration: {:?}", config);

    if !which::which("rg").is_ok() {
        tracing::error!("ripgrep (rg) is not installed or not in PATH. `search_code` tool will fail.");
        // Allow server to start, but search_code will error out.
    }

    let server_details = get_server_details();
    let handler = EnhancedServerHandler::new(config.clone());

    match config.transport_mode {
        TransportMode::Stdio => {
            tracing::info!("Using STDIO transport mode.");
            let transport_opts = TransportOptions::default();
            let transport = StdioTransport::new(transport_opts)?;
            let server_runtime = server_runtime::create_server(server_details, transport, handler);
            McpServer::start(&server_runtime).await?;
        }
        #[cfg(feature = "sse")]
        TransportMode::Sse => {
            tracing::info!(host = %config.sse_host, port = %config.sse_port, "Using SSE transport mode.");
            let sse_options = HyperServerOptions {
                host: config.sse_host.clone(),
                port: config.sse_port,
                enable_cors: true, // Example: enable CORS
                // ssl_config: None, // Add SSL config here if needed
                ..Default::default()
            };
            let sse_server_runtime = server_runtime_sse::create_server(server_details, handler, sse_options);
            McpServer::start(&sse_server_runtime).await?;
        }
        #[cfg(not(feature = "sse"))]
        TransportMode::Sse => {
            tracing::error!("SSE transport mode selected, but the 'sse' feature is not compiled. Falling back to STDIO.");
            // Fallback or error, for now, let's try stdio
            let transport_opts = TransportOptions::default();
            let transport = StdioTransport::new(transport_opts)?;
            let server_runtime = server_runtime::create_server(server_details, transport, handler);
            McpServer::start(&server_runtime).await?;
        }
    }

    tracing::info!("Server shutdown.");
    Ok(())
}