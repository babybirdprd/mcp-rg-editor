// This file's commands are now largely superseded by MCP tools.
// For this iteration, this file will be empty of commands, assuming MCP is the primary interface.
// If UI needs direct calls to process management logic not via MCP, define them here.
// Example:
/*
use crate::error::AppError;
use crate::utils::audit_logger::audit_log;
use crate::mcp::tool_impl::process as mcp_process_impl;
use crate::mcp::handler::ToolDependencies;
use crate::config::Config;

use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock as StdRwLock};
use tauri::{AppHandle, Manager, State};
use tokio::sync::Mutex as TokioMutex;
use tracing::instrument;

#[derive(Debug, Deserialize, Serialize)]
pub struct KillProcessParams {
    pub pid: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProcessInfo {
    pid: String,
    name: String,
    cpu_usage: f32,
    memory_mb: u64,
    command: String,
    status: String,
    user: Option<String>,
    start_time_epoch_secs: u64,
}

#[derive(Debug, Serialize)]
pub struct KillProcessResult {
    pub success: bool,
    pub message: String,
}

pub type SysinfoState = Arc<TokioMutex<sysinfo::System>>;

fn get_tool_dependencies_for_ui(app_handle: &AppHandle, config_state: &State<'_, Arc<StdRwLock<Config>>>) -> ToolDependencies {
    ToolDependencies {
        app_handle: app_handle.clone(),
        config_state: config_state.inner().clone(),
        audit_logger: app_handle.state::<Arc<crate::utils::audit_logger::AuditLogger>>().inner().clone(),
        fuzzy_search_logger: app_handle.state::<Arc<crate::utils::fuzzy_search_logger::FuzzySearchLogger>>().inner().clone(),
        active_sessions_map: app_handle.state::<crate::commands::terminal_commands::ActiveSessionsMap>().inner().clone(),
        sysinfo_state: app_handle.state::<SysinfoState>().inner().clone(),
    }
}

#[tauri::command(async)]
#[instrument(skip(app_handle, audit_logger_state, config_state, _sysinfo_state))]
pub async fn list_processes_command_ui_wrapper(
    app_handle: AppHandle,
    config_state: State<'_, Arc<StdRwLock<Config>>>,
    audit_logger_state: State<'_, Arc<crate::utils::audit_logger::AuditLogger>>,
    _sysinfo_state: State<'_, SysinfoState>,
) -> Result<Vec<ProcessInfo>, AppError> {
    audit_log(&audit_logger_state, "ui_list_processes_wrapper", &serde_json::Value::Null).await;
    let deps = get_tool_dependencies_for_ui(&app_handle, &config_state);
    mcp_process_impl::mcp_list_processes(&deps).await
        .map(|mcp_infos| {
            mcp_infos.into_iter().map(|mcp_info| {
                let mcp_info_json = serde_json::to_value(mcp_info).unwrap();
                serde_json::from_value(mcp_info_json).unwrap()
            }).collect()
        })
}

#[tauri::command(async)]
#[instrument(skip(app_handle, audit_logger_state, config_state, _sysinfo_state, params), fields(pid = %params.pid))]
pub async fn kill_process_command_ui_wrapper(
    app_handle: AppHandle,
    config_state: State<'_, Arc<StdRwLock<Config>>>,
    audit_logger_state: State<'_, Arc<crate::utils::audit_logger::AuditLogger>>,
    _sysinfo_state: State<'_, SysinfoState>,
    params: KillProcessParams,
) -> Result<KillProcessResult, AppError> {
    audit_log(&audit_logger_state, "ui_kill_process_wrapper", &serde_json::to_value(¶ms)?).await;
    let deps = get_tool_dependencies_for_ui(&app_handle, &config_state);
    let mcp_params = crate::mcp::tool_impl::process::KillProcessParamsMCP { pid: params.pid };
    mcp_process_impl::mcp_kill_process(&deps, mcp_params).await
        .map(|mcp_res| KillProcessResult { success: mcp_res.success, message: mcp_res.message })
}
*/