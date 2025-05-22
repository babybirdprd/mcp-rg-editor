// FILE: src-tauri/src/mcp/tool_impl/terminal.rs
use crate::config::Config;
use crate::error::AppError;
use crate::mcp::handler::ToolDependencies;
use crate::commands::terminal_commands::ActiveSession; // Use the existing ActiveSession

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, Runtime}; // Added Emitter
use tauri_plugin_shell::{process::CommandEvent, ShellExt}; // Corrected CommandEvent import
use tokio::sync::Mutex as TokioMutex;
use tokio::time::{timeout, Duration, Instant as TokioInstant}; // Use TokioInstant
use tracing::{debug, error, info, instrument, warn};
use uuid::Uuid;
use chrono::Utc;
use serde_json::json; // For json! macro

// --- MCP Specific Parameter Structs ---
#[derive(Debug, Deserialize)]
pub struct ExecuteCommandParamsMCP {
    pub command: String,
    #[serde(rename = "timeout_ms")]
    pub timeout_ms: Option<u64>,
    pub shell: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ForceTerminateParamsMCP { pub session_id: String } // Corrected syntax
#[derive(Debug, Deserialize)]
pub struct ReadOutputStatusParamsMCP { pub session_id: String } // Corrected syntax

// --- MCP Specific Result Structs ---
#[derive(Debug, Serialize)]
pub struct ExecuteCommandResultMCP {
    pub session_id: String,
    pub pid: Option<u32>,
    pub initial_output: String,
    pub timed_out: bool,
    pub exit_code: Option<i32>,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct ForceTerminateResultMCP { pub session_id: String, pub success: bool, pub message: String } // Corrected syntax
#[derive(Debug, Serialize)]
pub struct SessionInfoMCP { pub session_id: String, pub command: String, pub pid: Option<u32>, pub is_running: bool, pub start_time_iso: String, pub runtime_ms: u128 }
#[derive(Debug, Serialize)]
pub struct ReadOutputStatusResultMCP { pub session_id: String, pub is_running: bool, pub exit_code: Option<i32>, pub message: String, pub recent_output: Option<String> }


fn is_command_blocked_mcp(command_str: &str, config: &Config) -> bool {
    let first_command_word = command_str.trim_start().split_whitespace().next().unwrap_or("");
    if first_command_word.is_empty() { return false; }
    match config.get_blocked_command_regexes() {
        Ok(regexes) => regexes.iter().any(|regex| regex.is_match(first_command_word)),
        Err(e) => { warn!("Error compiling blocked command regexes: {}. Blocking {} as precaution.", e, first_command_word); config.blocked_commands.iter().any(|b| b == first_command_word)}
    }
}

#[instrument(skip(deps, params), fields(command = %params.command))]
pub async fn mcp_execute_command(deps: &ToolDependencies, params: ExecuteCommandParamsMCP) -> Result<ExecuteCommandResultMCP, AppError> {
    let config_guard = deps.config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock: {}", e)))?;
    if is_command_blocked_mcp(params.command, &config_guard) {
        return Err(AppError::CommandBlocked(params.command.clone()));
    }
    let cwd_path = config_guard.files_root.clone();
    let shell_to_use = params.shell.clone().or_else(|| config_guard.default_shell.clone());
    drop(config_guard);

    let session_id = Uuid::new_v4().to_string();
    let (mut command_obj, shell_args_for_scope_check): (tauri_plugin_shell::Command, Vec<String>) = if let Some(shell_path) = &shell_to_use {
        let mut args = Vec::new();
        if shell_path.contains("powershell") || shell_path.contains("cmd.exe") { args.push("-Command".to_string()); } else { args.push("-c".to_string()); }
        args.push(params.command.clone());
        (deps.app_handle.shell().command(shell_path.clone()).args(args.clone()), args)
    } else {
        let mut parts = params.command.split_whitespace();
        let prog = parts.next().ok_or_else(|| AppError::CommandExecutionError("Empty command".into()))?;
        let args: Vec<String> = parts.map(String::from).collect();
        (deps.app_handle.shell().command(prog.to_string()).args(args.clone()), args)
    };
    
    // No direct set_stdin on tauri_plugin_shell::Command. Stdin(null) is default for non-interactive.
    command_obj = command_obj.current_dir(cwd_path);
    // command_obj = command_obj.set_stdin(std::process::Stdio::null()); // This was incorrect for tauri_plugin_shell::Command

    let program_name_for_scope_check = command_obj.program_for_scope_check().to_string_lossy().to_string(); // Helper method needed or store program name
    if !deps.app_handle.shell().scope().is_allowed(&program_name_for_scope_check, &shell_args_for_scope_check) {
        return Err(AppError::CommandBlocked(format!("Execution of '{}' not permitted by shell scope.", program_name_for_scope_check)));
    }

    debug!(shell = ?shell_to_use, command = %params.command, "MCP Tool: Spawning command via tauri-plugin-shell");
    let (mut rx, child_proc_handle) = command_obj.spawn().map_err(|e| AppError::CommandExecutionError(format!("Spawn failed: {}", e)))?;
    let pid_val = child_proc_handle.pid();

    let active_session_arc = Arc::new(ActiveSession {
        process_child: Arc::new(TokioMutex::new(child_proc_handle)),
        command_str: params.command.clone(),
        exit_code: Arc::new(TokioMutex::new(None)),
        start_time_system: std::time::SystemTime::now(),
        session_id: session_id.clone(),
        pid: Some(pid_val), // pid is u32
    });
    deps.active_sessions_map.lock().await.insert(session_id.clone(), active_session_arc.clone());

    let initial_output_timeout_ms = params.timeout_ms.unwrap_or(1000);
    let mut initial_stdout_lines = Vec::new();
    let mut initial_stderr_lines = Vec::new();
    let mut timed_out_flag = false;
    let mut early_exit_code: Option<i32> = None;

    let output_collection_start_time = TokioInstant::now();
    loop {
        if output_collection_start_time.elapsed() > Duration::from_millis(initial_output_timeout_ms) {
            if early_exit_code.is_none() { timed_out_flag = true; }
            break;
        }
        match timeout(Duration::from_millis(50), rx.recv()).await {
            Ok(Ok(Some(event))) => match event {
                CommandEvent::Stdout(line) => initial_stdout_lines.push(String::from_utf8_lossy(&line).into_owned()),
                CommandEvent::Stderr(line) => initial_stderr_lines.push(String::from_utf8_lossy(&line).into_owned()),
                CommandEvent::Terminated(payload) => { early_exit_code = payload.code; break; }
                CommandEvent::Error(msg) => { error!("Cmd error during initial read: {}", msg); early_exit_code = Some(-1); break; }
                _ => {}
            },
            Ok(Ok(None)) => { break; } 
            Ok(Err(e)) => { error!("rx.recv error: {:?}", e); early_exit_code = Some(-2); break; } 
            Err(_) => { /* timeout for this 50ms iteration */ }
        }
    }
    
    let combined_initial_output = format!("STDOUT:\n{}\nSTDERR:\n{}", initial_stdout_lines.join("\n"), initial_stderr_lines.join("\n"));

    let app_handle_clone = deps.app_handle.clone();
    let session_id_clone_for_task = session_id.clone();
    let active_session_clone_for_task = active_session_arc.clone();
    let sessions_state_clone_for_task = deps.active_sessions_map.clone();

    if early_exit_code.is_none() {
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Some(CommandEvent::Stdout(line)) => {
                        app_handle_clone.emit_to("main", &format!("terminal_output_{}", session_id_clone_for_task), json!({"type": "stdout", "data": String::from_utf8_lossy(&line).into_owned()})).unwrap_or_else(|e| error!("Emit stdout failed: {}", e));
                    }
                    Some(CommandEvent::Stderr(line)) => {
                        app_handle_clone.emit_to("main", &format!("terminal_output_{}", session_id_clone_for_task), json!({"type": "stderr", "data": String::from_utf8_lossy(&line).into_owned()})).unwrap_or_else(|e| error!("Emit stderr failed: {}", e));
                    }
                    Some(CommandEvent::Terminated(payload)) => {
                        info!(sid = %session_id_clone_for_task, code = ?payload.code, "Background task: Command terminated");
                        *active_session_clone_for_task.exit_code.lock().await = payload.code;
                        app_handle_clone.emit_to("main", &format!("terminal_output_{}", session_id_clone_for_task), json!({"type": "terminated", "code": payload.code, "signal": payload.signal })).unwrap_or_else(|e| error!("Emit terminated failed: {}", e));
                        sessions_state_clone_for_task.lock().await.remove(&session_id_clone_for_task);
                        break;
                    }
                    Some(CommandEvent::Error(message)) => {
                        error!(sid = %session_id_clone_for_task, message = %message, "Background task: Command error in stream");
                        *active_session_clone_for_task.exit_code.lock().await = Some(-1);
                        app_handle_clone.emit_to("main", &format!("terminal_output_{}", session_id_clone_for_task), json!({"type": "error", "data": message })).unwrap_or_else(|e| error!("Emit error failed: {}", e));
                        sessions_state_clone_for_task.lock().await.remove(&session_id_clone_for_task);
                        break;
                    }
                    None => {
                        info!(sid = %session_id_clone_for_task, "Background task: Command event stream closed");
                        if active_session_clone_for_task.exit_code.lock().await.is_none() {
                            *active_session_clone_for_task.exit_code.lock().await = Some(0);
                        }
                        app_handle_clone.emit_to("main", &format!("terminal_output_{}", session_id_clone_for_task), json!({"type": "finished_stream_closed"})).unwrap_or_else(|e| error!("Emit finished failed: {}", e));
                        sessions_state_clone_for_task.lock().await.remove(&session_id_clone_for_task);
                        break;
                    }
                }
            }
            info!(sid = %session_id_clone_for_task, "Exiting command monitoring background task.");
        });
    } else {
        *active_session_arc.exit_code.lock().await = early_exit_code;
        deps.active_sessions_map.lock().await.remove(&session_id);
    }

    let final_exit_code = *active_session_arc.exit_code.lock().await;
    let message = if timed_out_flag && final_exit_code.is_none() { format!("Cmd started (PID:{:?}, SID:{}), timed out for initial output. Output streamed via events.", pid_val, session_id) }
                  else if final_exit_code.is_none() { format!("Cmd running (PID:{:?}, SID:{}). Output streamed via events.", pid_val, session_id) }
                  else { format!("Cmd finished (PID:{:?}, SID:{}). Exit: {:?}.", pid_val, session_id, final_exit_code) };

    Ok(ExecuteCommandResultMCP { session_id, pid: Some(pid_val), initial_output: combined_initial_output, timed_out: timed_out_flag, exit_code: final_exit_code, message })
}

pub async fn mcp_force_terminate_session(deps: &ToolDependencies, params: ForceTerminateParamsMCP) -> Result<ForceTerminateResultMCP, AppError> {
    let session_id = params.session_id;
    if let Some(session_arc) = deps.active_sessions_map.lock().await.get(&session_id).cloned() {
        let mut child_guard = session_arc.process_child.lock().await;
        match child_guard.kill() {
            Ok(_) => {
                info!(sid = %session_id, pid = ?session_arc.pid, "MCP Tool: Termination signal sent.");
                *session_arc.exit_code.lock().await = Some(-9);
                deps.active_sessions_map.lock().await.remove(&session_id);
                Ok(ForceTerminateResultMCP { session_id, success: true, message: "Termination signal sent.".into() })
            }
            Err(e) => {
                warn!(sid = %session_id, pid = ?session_arc.pid, error = %e, "MCP Tool: Failed to send kill signal");
                if session_arc.exit_code.lock().await.is_none() { *session_arc.exit_code.lock().await = Some(-10); }
                Ok(ForceTerminateResultMCP { session_id, success: false, message: format!("Kill signal failed: {}", e) })
            }
        }
    } else { Err(AppError::SessionNotFound(session_id)) }
}

pub async fn mcp_list_sessions(deps: &ToolDependencies) -> Result<Vec<SessionInfoMCP>, AppError> {
    let sessions_map = deps.active_sessions_map.lock().await;
    let mut infos = Vec::new();
    let now_sys = std::time::SystemTime::now();
    for (id, session) in sessions_map.iter() {
        let exit_code = *session.exit_code.lock().await;
        infos.push(SessionInfoMCP {
            session_id: id.clone(), command: session.command_str.clone(), pid: session.pid,
            is_running: exit_code.is_none(), start_time_iso: chrono::DateTime::<Utc>::from(session.start_time_system).to_rfc3339(),
            runtime_ms: now_sys.duration_since(session.start_time_system).unwrap_or_default().as_millis(),
        });
    }
    Ok(infos)
}

pub async fn mcp_read_session_output_status(deps: &ToolDependencies, params: ReadOutputStatusParamsMCP) -> Result<ReadOutputStatusResultMCP, AppError> {
    let session_id = params.session_id;
    if let Some(session_arc) = deps.active_sessions_map.lock().await.get(&session_id).cloned() {
        let exit_code = *session_arc.exit_code.lock().await;
        Ok(ReadOutputStatusResultMCP {
            session_id, is_running: exit_code.is_none(), exit_code,
            message: "Session status. For UI, output is streamed via Tauri events. MCP client cannot directly access this stream without further adaptation.".into(),
            recent_output: None
        })
    } else {
        Ok(ReadOutputStatusResultMCP { session_id, is_running: false, exit_code: None, message: "Session not found or already terminated.".into(), recent_output: None })
    }
}