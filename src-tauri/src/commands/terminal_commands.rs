// This file's commands are now largely superseded by MCP tools.
// The ActiveSession and ActiveSessionsMap types are still needed by the MCP terminal tool implementation.
// If UI needs direct calls to terminal logic not via MCP, define them here.
// For this iteration, this file will only contain the necessary type definitions.

use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tauri_plugin_shell::process::CommandChild;
use tokio::sync::Mutex as TokioMutex; // Keep TokioMutex for ActiveSession

#[derive(Debug, Clone, Serialize)]
pub struct ExecuteCommandResultUI { 
    pub session_id: String,
    pub pid: Option<u32>,
    pub message: String,
}

#[derive(Debug)]
pub struct ActiveSession {
    pub process_child: Arc<TokioMutex<Option<CommandChild>>>,
    pub command_str: String,
    pub exit_code: Arc<TokioMutex<Option<i32>>>,
    pub start_time_system: std::time::SystemTime,
    #[allow(dead_code)] // session_id is used as key in map and for SessionInfoMCP, but not read directly from ActiveSession instance itself
    pub session_id: String,
    pub pid: Option<u32>,
}

pub type ActiveSessionsMap = Arc<TokioMutex<HashMap<String, Arc<ActiveSession>>>>;

// UI-specific Tauri commands for terminal operations (e.g., execute_command_ui, 
// force_terminate_session_ui, list_sessions_ui, read_session_output_status_ui)
// would go here if they were not superseded by MCP tools.
// For now, they are removed to focus on the MCP implementation.