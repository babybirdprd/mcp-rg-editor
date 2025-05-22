// This file's commands are now largely superseded by MCP tools.
// For this iteration, this file will be empty of commands, assuming MCP is the primary interface.
// If UI needs direct calls to ripgrep logic not via MCP, define them here.
// Example:
/*
use crate::config::Config;
use crate::error::AppError;
use crate::utils::audit_logger::audit_log;
use crate::mcp::tool_impl::ripgrep as mcp_rg_impl;
use crate::mcp::handler::ToolDependencies;

use std::sync::{Arc, RwLock as StdRwLock};
use tauri::{AppHandle, Manager, State};
use sysinfo::System as SysinfoSystem;
use tokio::sync::Mutex as TokioMutex;
use tracing::instrument;

pub use crate::mcp::tool_impl::ripgrep::SearchCodeParamsMCP as SearchCodeParams;
pub use crate::mcp::tool_impl::ripgrep::SearchCodeResultMCP as SearchCodeResult;

fn get_tool_dependencies_for_ui(app_handle: &AppHandle, config_state: &State<'_, Arc<StdRwLock<Config>>>) -> ToolDependencies {
    ToolDependencies {
        app_handle: app_handle.clone(),
        config_state: config_state.inner().clone(),
        audit_logger: app_handle.state::<Arc<crate::utils::audit_logger::AuditLogger>>().inner().clone(),
        fuzzy_search_logger: app_handle.state::<Arc<crate::utils::fuzzy_search_logger::FuzzySearchLogger>>().inner().clone(),
        active_sessions_map: app_handle.state::<crate::commands::terminal_commands::ActiveSessionsMap>().inner().clone(),
        sysinfo_state: app_handle.state::<Arc<TokioMutex<SysinfoSystem>>>().inner().clone(),
    }
}

#[tauri::command(async)]
#[instrument(skip(app_handle, config_state, audit_logger_state, params), fields(pattern = %params.pattern, path = %params.path))]
pub async fn search_code_command_ui_wrapper( // Example wrapper
    app_handle: AppHandle,
    config_state: State<'_, Arc<StdRwLock<Config>>>,
    audit_logger_state: State<'_, Arc<crate::utils::audit_logger::AuditLogger>>,
    params: SearchCodeParams,
) -> Result<SearchCodeResult, AppError> {
    audit_log(&audit_logger_state, "ui_search_code_wrapper", &serde_json::to_value(¶ms)?).await;
    let deps = get_tool_dependencies_for_ui(&app_handle, &config_state);
    mcp_rg_impl::mcp_search_code(&deps, params).await
}
*/