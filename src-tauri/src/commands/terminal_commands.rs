// FILE: src-tauri/src/commands/terminal_commands.rs
use crate::config::Config;
use crate::error::AppError;
use crate::utils::audit_logger::audit_log;
// use crate::utils::path_utils::validate_and_normalize_path; // Not used directly here, CWD is fixed

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
// use std::path::PathBuf; // Not used
use std::sync::{Arc, RwLock as StdRwLock};
use tauri::{AppHandle, Emitter, Manager, State, Runtime}; // Added Emitter
use tauri_plugin_shell::{process::CommandEvent, process::CommandChild, ShellExt}; // Corrected imports
use tokio::sync::Mutex as TokioMutex;
use tokio::time::{timeout, Duration, Instant as TokioInstant}; // Use TokioInstant
use tracing::{debug, error, info, instrument, warn};
use uuid::Uuid;
use chrono::Utc;
use serde_json::json; // For json! macro

// --- Request Structs (for UI commands, if any; MCP uses its own in mcp::tool_impl) ---
// These might be identical to MCP ones if UI calls same logic, or different if UI has simpler needs.
// For now, assuming MCP tool_impl structs are primary and UI might have simpler wrappers if needed.

// --- Response Structs (for UI commands) ---
#[derive(Debug, Clone, Serialize)] // Added Clone
pub struct ExecuteCommandResultUI { // Renamed to avoid conflict if MCP one is different
    pub session_id: String,
    pub pid: Option<u32>,
    pub message: String, // UI might only need a simple message, details via events
}

// --- Internal Session Management ---
#[derive(Debug)]
pub struct ActiveSession {
    pub process_child: Arc<TokioMutex<CommandChild>>, // Made pub
    pub command_str: String,                          // Made pub
    pub exit_code: Arc<TokioMutex<Option<i32>>>,      // Made pub
    pub start_time_system: std::time::SystemTime,     // Made pub
    pub session_id: String,                           // Made pub
    pub pid: Option<u32>,                             // Made pub
}

pub type ActiveSessionsMap = Arc<TokioMutex<HashMap<String, Arc<ActiveSession>>>>;

// This function is now internal to the terminal command logic, not exposed as a Tauri command
// It's called by the MCP execute_command_mcp via ToolDependencies
fn is_command_blocked_internal(command_str: &str, config_guard: &Config) -> bool {
    let first_command_word = command_str.trim_start().split_whitespace().next().unwrap_or("");
    if first_command_word.is_empty() { return false; }

    match config_guard.get_blocked_command_regexes() {
        Ok(regexes) => regexes.iter().any(|regex| regex.is_match(first_command_word)),
        Err(e) => {
            warn!("Error compiling blocked command regexes: {}. Blocking command {} as a precaution.", e, first_command_word);
            config_guard.blocked_commands.iter().any(|blocked| blocked == first_command_word)
        }
    }
}

// Note: The `execute_command`, `force_terminate_session`, `list_sessions`, 
// and `read_session_output_status` Tauri commands are effectively superseded by the MCP tool implementations.
// If the UI needs to *also* call these directly (not via MCP), then these Tauri commands would be kept
// and would likely call the same underlying logic now in `src-tauri/src/mcp/tool_impl/terminal.rs`.
// For now, I'm commenting them out to avoid duplication and focus on MCP path.
// If UI-specific versions are needed, they can be reinstated.

/*
#[tauri::command(async)]
#[instrument(skip(app_handle, config_state, audit_logger_state, sessions_state, params), fields(command = %params.command))]
pub async fn execute_command_ui( // Renamed to avoid conflict
    app_handle: AppHandle,
    config_state: State<'_, Arc<StdRwLock<Config>>>,
    audit_logger_state: State<'_, Arc<crate::utils::audit_logger::AuditLogger>>,
    sessions_state: State<'_, ActiveSessionsMap>,
    params: ExecuteCommandParams, // Assuming this is a UI-specific param struct
) -> Result<ExecuteCommandResultUI, AppError> {
    // ... implementation similar to mcp_execute_command but tailored for UI events/responses ...
    // This would involve spawning the process and setting up event listeners
    // that emit to the frontend.
    // For simplicity, this example will be brief.
    audit_log(&audit_logger_state, "ui_execute_command", &serde_json::to_value(&params)?).await;
    
    // Placeholder:
    Ok(ExecuteCommandResultUI {
        session_id: Uuid::new_v4().to_string(),
        pid: None,
        message: "Command execution initiated. Listen for events.".to_string(),
    })
}

#[tauri::command(async)]
#[instrument(skip(audit_logger_state, sessions_state, params), fields(session_id = %params.session_id))]
pub async fn force_terminate_session_ui( // Renamed
    audit_logger_state: State<'_, Arc<crate::utils::audit_logger::AuditLogger>>,
    sessions_state: State<'_, ActiveSessionsMap>,
    params: ForceTerminateParams, // UI-specific
) -> Result<ForceTerminateResult, AppError> {
    audit_log(&audit_logger_state, "ui_force_terminate_session", &serde_json::to_value(&params)?).await;
    // ... similar logic to mcp_force_terminate_session ...
    Ok(ForceTerminateResult {
        session_id: params.session_id,
        success: true, // Placeholder
        message: "Termination signal sent (UI placeholder).".to_string(),
    })
}

#[tauri::command(async)]
#[instrument(skip(audit_logger_state, sessions_state))]
pub async fn list_sessions_ui( // Renamed
    audit_logger_state: State<'_, Arc<crate::utils::audit_logger::AuditLogger>>,
    sessions_state: State<'_, ActiveSessionsMap>,
) -> Result<Vec<SessionInfo>, AppError> {
    audit_log(&audit_logger_state, "ui_list_sessions", &serde_json::Value::Null).await;
    // ... similar logic to mcp_list_sessions ...
    Ok(vec![]) // Placeholder
}


#[tauri::command(async)]
#[instrument(skip(audit_logger_state, sessions_state, params), fields(session_id = %params.session_id))]
pub async fn read_session_output_status_ui( // Renamed
    audit_logger_state: State<'_, Arc<crate::utils::audit_logger::AuditLogger>>,
    sessions_state: State<'_, ActiveSessionsMap>,
    params: ReadOutputStatusParams, // UI-specific
) -> Result<ReadOutputStatusResult, AppError> {
    audit_log(&audit_logger_state, "ui_read_session_output_status", &serde_json::to_value(&params)?).await;
    // ... similar logic to mcp_read_session_output_status ...
    Ok(ReadOutputStatusResult { // Placeholder
        session_id: params.session_id,
        is_running: false,
        exit_code: None,
        message: "Status retrieved (UI placeholder).".to_string(),
    })
}
*/