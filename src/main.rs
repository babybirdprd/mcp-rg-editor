// FILE: src/main.rs
mod config;
mod error;
mod mcp;
mod tools;
mod utils;

use crate::config::{Config, TransportMode};
use crate::mcp::handler::EnhancedServerHandler;
use anyhow::Result;
use rust_mcp_schema::{InitializeResult, Implementation, ServerCapabilities, ServerCapabilitiesTools, LATEST_PROTOCOL_VERSION};
use rust_mcp_transport::{StdioTransport, TransportOptions};
use rust_mcp_sdk::error::McpSdkError;
use tracing::Level;
use tracing_subscriber::{filter::EnvFilter, FmtSubscriber, fmt::format::FmtSpan};

#[cfg(feature = "sse")]
use rust_mcp_sdk::mcp_server::hyper_server::{create_server as create_sse_server, HyperServerOptions, HyperServer}; // Corrected import path
#[cfg(feature = "stdio")]
use rust_mcp_sdk::mcp_server::server_runtime::{create_server as create_stdio_server, ServerRuntime}; // Corrected import path


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
        .unwrap_or_else(|_| EnvFilter::new(format!("mcp_rg_editor={}", level)));

    let subscriber = FmtSubscriber::builder()
        .with_env_filter(env_filter)
        .with_target(true)
        .with_ansi(false) 
        .with_writer(std::io::stderr)
        .with_level(true)
        .with_span_events(FmtSpan::CLOSE) 
        .json() 
        .finish();

    tracing::subscriber::set_global_default(subscriber)
        .expect("Failed to set tracing subscriber");
}

fn get_server_details() -> InitializeResult {
    InitializeResult {
        server_info: Implementation {
            name: "mcp-rg-editor-desktop-commander-enhanced".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
        capabilities: ServerCapabilities {
            tools: Some(ServerCapabilitiesTools { list_changed: None }),
            resources: Some(Default::default()), 
            prompts: Some(Default::default()),   
            ..Default::default()
        },
        meta: None,
        instructions: Some(
            "Enhanced MCP Server with Desktop Commander features: Ripgrep, Filesystem, Terminal, Process, and Editing operations. \
            All paths should be absolute or tilde-expanded (~/...). Relative paths are resolved against FILES_ROOT. \
            Use `get_config` to see current FILES_ROOT and other settings. \
            For `write_file` and `edit_block`, respect `fileWriteLineLimit` and chunk large changes.".to_string()
        ),
        protocol_version: LATEST_PROTOCOL_VERSION.to_string(),
    }
}

async fn map_mcp_sdk_error(err: McpSdkError) -> anyhow::Error {
    anyhow::anyhow!("MCP SDK Error: {:?}", err)
}

#[tokio::main]
async fn main() -> Result<()> {
    let initial_config = Config::load().expect("Failed to load initial configuration.");
    setup_logging(&initial_config.log_level);

    tracing::info!(version = %env!("CARGO_PKG_VERSION"), "Starting mcp-rg-editor (Desktop Commander Enhanced) server");
    tracing::debug!("Loaded initial configuration: {:?}", initial_config);

    if which::which("rg").is_err() {
        tracing::warn!("ripgrep (rg) is not installed or not in PATH. `search_code` tool will fail if rg is not found at runtime.");
    }

    let server_details = get_server_details();
    let handler = EnhancedServerHandler::new(initial_config.clone());

    match initial_config.transport_mode {
        #[cfg(feature = "stdio")]
        TransportMode::Stdio => {
            tracing::info!("Using STDIO transport mode.");
            let transport_opts = TransportOptions::default();
            let transport = StdioTransport::new(transport_opts)
                .map_err(|e| anyhow::anyhow!("Failed to create StdioTransport: {}", e))?;
            let server_runtime: ServerRuntime<_> = create_stdio_server(server_details, transport, handler);
            server_runtime.start().await.map_err(map_mcp_sdk_error).await?;
        }
        #[cfg(feature = "sse")]
        TransportMode::Sse => {
            tracing::info!(host = %initial_config.sse_host, port = %initial_config.sse_port, "Using SSE transport mode.");
            let sse_options = HyperServerOptions {
                host: initial_config.sse_host.clone(),
                port: initial_config.sse_port,
                enable_cors: true, 
                ..Default::default()
            };
            let sse_server_runtime: HyperServer<_> = create_sse_server(server_details, handler, sse_options);
            sse_server_runtime.start().await.map_err(map_mcp_sdk_error).await?;
        }
        #[cfg(not(all(feature = "stdio", feature = "sse")))] 
        #[allow(unreachable_patterns)] 
        _ => {
            let available_feature = if cfg!(feature = "stdio") { "stdio" } else if cfg!(feature = "sse") { "sse" } else { "none" };
            tracing::error!(
                selected_transport = ?initial_config.transport_mode,
                available_feature = %available_feature,
                "Selected transport mode is not available due to compiled features."
            );
            anyhow::bail!(
                "Selected transport mode {:?} is not available. Compiled with {} support only.",
                initial_config.transport_mode, available_feature
            );
        }
    }

    tracing::info!("Server shutdown.");
    Ok(())
}