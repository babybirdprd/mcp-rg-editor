use crate::config::Config;
use crate::error::AppError;
use crate::utils::audit_logger::audit_log;
use crate::utils::path_utils::validate_and_normalize_path;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock as StdRwLock};
use tauri::{AppHandle, Manager, State, Runtime};
use tauri_plugin_shell::{ShellExt, Command, CommandEvent, process::CommandChild}; // Corrected import
use tokio::sync::Mutex as TokioMutex;
use tokio::time::{timeout, Duration};
use tracing::{debug, error, info, instrument, warn};
use uuid::Uuid;
use chrono::Utc;

// --- Request Structs ---
#[derive(Debug, Deserialize, Serialize)]
pub struct ExecuteCommandParams {
    pub command: String,
    #[serde(rename = "timeout_ms")]
    pub timeout_ms: Option<u64>,
    pub shell: Option<String>,
    // pub working_directory: Option<String>, // Consider adding this
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ForceTerminateParams {
    pub session_id: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ReadOutputStatusParams {
    pub session_id: String,
}


// --- Response Structs ---
#[derive(Debug, Serialize)]
pub struct ExecuteCommandResult {
    pub session_id: String,
    pub pid: Option<u32>,
    pub initial_output: String,
    pub timed_out: bool,
    pub exit_code: Option<i32>,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct ForceTerminateResult {
    pub session_id: String,
    pub success: bool,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct SessionInfo {
    pub session_id: String,
    pub command: String,
    pub pid: Option<u32>,
    pub is_running: bool,
    pub start_time_iso: String,
    pub runtime_ms: u128,
}

#[derive(Debug, Serialize)]
pub struct ReadOutputStatusResult {
    pub session_id: String,
    pub is_running: bool,
    pub exit_code: Option<i32>,
    pub message: String, // e.g., "Output is streamed via events."
}


// --- Internal Session Management ---
#[derive(Debug)]
pub struct ActiveSession { // Made pub for access from lib.rs if needed for state init
    process_child: Arc<TokioMutex<CommandChild>>,
    command_str: String,
    exit_code: Arc<TokioMutex<Option<i32>>>,
    start_time_system: std::time::SystemTime,
    session_id: String,
    pid: Option<u32>,
}

pub type ActiveSessionsMap = Arc<TokioMutex<HashMap<String, Arc<ActiveSession>>>>;


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

#[tauri::command(async)]
#[instrument(skip(app_handle, config_state, audit_logger_state, sessions_state, params), fields(command = %params.command))]
pub async fn execute_command(
    app_handle: AppHandle,
    config_state: State<'_, Arc<StdRwLock<Config>>>,
    audit_logger_state: State<'_, Arc<crate::utils::audit_logger::AuditLogger>>,
    sessions_state: State<'_, ActiveSessionsMap>,
    params: ExecuteCommandParams,
) -> Result<ExecuteCommandResult, AppError> {
    audit_log(&audit_logger_state, "execute_command", &serde_json::to_value(params)?).await;
    let config_guard = config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock error: {}",e)))?;

    if is_command_blocked_internal(params.command, &config_guard) {
        warn!(command = %params.command, "Command execution blocked by configuration");
        return Err(AppError::CommandBlocked(params.command.clone()));
    }

    let files_root_for_cwd = config_guard.files_root.clone();
    let cwd_path = files_root_for_cwd; // Simplified: always use files_root as CWD for now

    let shell_to_use = params.shell.clone().or_else(|| config_guard.default_shell.clone());
    drop(config_guard);

    let session_id = Uuid::new_v4().to_string();
    let (mut command_obj, shell_args_for_scope_check): (Command, Vec<String>) = if let Some(shell_path_str) = &shell_to_use {
        let mut args = Vec::new();
        if shell_path_str.contains("powershell") || shell_path_str.contains("cmd.exe") {
             args.push("-Command".to_string());
        } else {
             args.push("-c".to_string());
        }
        args.push(params.command.clone());
        (app_handle.shell().command(shell_path_str.clone()).args(args.clone()), args) // Pass args for scope check
    } else {
        let mut parts = params.command.split_whitespace();
        let program = parts.next().ok_or_else(|| AppError::CommandExecutionError("Empty command".to_string()))?;
        let args: Vec<String> = parts.map(String::from).collect();
        (app_handle.shell().command(program.to_string()).args(args.clone()), args) // Pass args for scope check
    };

    command_obj = command_obj.current_dir(cwd_path);
    command_obj = command_obj.set_stdin(tauri_plugin_shell::process::Stdio::Null);

    let program_name_for_scope_check = command_obj.get_program().to_string_lossy().to_string();
    if !app_handle.shell().scope().is_allowed(&program_name_for_scope_check, &shell_args_for_scope_check) {
        warn!(command = %program_name_for_scope_check, args = ?shell_args_for_scope_check, "Command execution not allowed by shell scope.");
        return Err(AppError::CommandBlocked(format!(
            "Execution of '{}' with given arguments is not permitted by shell scope.",
            program_name_for_scope_check
        )));
    }

    debug!(shell = ?shell_to_use, command = %params.command, "Spawning command via tauri-plugin-shell");
    let (mut rx, child_process) = command_obj.spawn().map_err(|e| {
        error!(error = %e, command = %params.command, "Failed to spawn command via plugin");
        AppError::CommandExecutionError(format!("Failed to spawn command '{}': {}", params.command, e))
    })?;

    let pid = child_process.pid();
    let command_str_clone = params.command.clone();

    let active_session_arc = Arc::new(ActiveSession {
        process_child: Arc::new(TokioMutex::new(child_process)),
        command_str: command_str_clone,
        exit_code: Arc::new(TokioMutex::new(None::<i32>)),
        start_time_system: std::time::SystemTime::now(),
        session_id: session_id.clone(),
        pid,
    });

    sessions_state.lock().await.insert(session_id.clone(), active_session_arc.clone());

    let initial_output_timeout = Duration::from_millis(params.timeout_ms.unwrap_or(1000));
    let mut timed_out_flag = false;

    let app_handle_clone = app_handle.clone();
    let session_id_clone_for_task = session_id.clone();
    let active_session_clone_for_task = active_session_arc.clone();
    let sessions_state_clone_for_task = sessions_state.clone();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                maybe_event = rx.recv() => {
                    match maybe_event {
                        Some(CommandEvent::Stdout(line)) => {
                            let line_str = String::from_utf8_lossy(&line).into_owned();
                            app_handle_clone.emit_to("main", &format!("terminal_output_{}", session_id_clone_for_task), serde_json::json!({"type": "stdout", "data": line_str })).unwrap_or_else(|e| error!("Emit stdout failed: {}", e));
                        }
                        Some(CommandEvent::Stderr(line)) => {
                            let line_str = String::from_utf8_lossy(&line).into_owned();
                             app_handle_clone.emit_to("main", &format!("terminal_output_{}", session_id_clone_for_task), serde_json::json!({"type": "stderr", "data": line_str })).unwrap_or_else(|e| error!("Emit stderr failed: {}", e));
                        }
                        Some(CommandEvent::Terminated(payload)) => {
                            info!(sid = %session_id_clone_for_task, code = ?payload.code, "Command terminated");
                            *active_session_clone_for_task.exit_code.lock().await = payload.code;
                             app_handle_clone.emit_to("main", &format!("terminal_output_{}", session_id_clone_for_task), serde_json::json!({"type": "terminated", "code": payload.code, "signal": payload.signal })).unwrap_or_else(|e| error!("Emit terminated failed: {}", e));
                            sessions_state_clone_for_task.lock().await.remove(&session_id_clone_for_task);
                            break;
                        }
                        Some(CommandEvent::Error(message)) => {
                            error!(sid = %session_id_clone_for_task, message = %message, "Command error in stream");
                            *active_session_clone_for_task.exit_code.lock().await = Some(-1);
                             app_handle_clone.emit_to("main", &format!("terminal_output_{}", session_id_clone_for_task), serde_json::json!({"type": "error", "data": message })).unwrap_or_else(|e| error!("Emit error failed: {}", e));
                            sessions_state_clone_for_task.lock().await.remove(&session_id_clone_for_task);
                            break;
                        }
                        None => {
                            info!(sid = %session_id_clone_for_task, "Command event stream closed");
                            if active_session_clone_for_task.exit_code.lock().await.is_none() {
                                *active_session_clone_for_task.exit_code.lock().await = Some(0); // Assume success if stream closes cleanly
                            }
                            app_handle_clone.emit_to("main", &format!("terminal_output_{}", session_id_clone_for_task), serde_json::json!({"type": "finished_stream_closed"})).unwrap_or_else(|e| error!("Emit finished failed: {}", e));
                            sessions_state_clone_for_task.lock().await.remove(&session_id_clone_for_task);
                            break;
                        }
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(50)) => {} // Avoid tight loop on continuous None
            }
        }
        info!(sid = %session_id_clone_for_task, "Exiting command monitoring task.");
    });

    let start_initial_wait = std::time::Instant::now();
    while start_initial_wait.elapsed() < initial_output_timeout {
        if active_session_arc.exit_code.lock().await.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let final_exit_code = *active_session_arc.exit_code.lock().await;
    timed_out_flag = final_exit_code.is_none() && start_initial_wait.elapsed() >= initial_output_timeout;

    let message = if timed_out_flag {
        format!("Command started (PID: {:?}, Session: {}). Output is streamed. Timed out waiting for initial exit.", pid, session_id)
    } else if final_exit_code.is_none() {
        format!("Command running (PID: {:?}, Session: {}). Output is streamed.", pid, session_id)
    } else {
        format!("Command finished (PID: {:?}, Session: {}). Exit code: {:?}.", pid, session_id, final_exit_code)
    };

    if final_exit_code.is_some() {
        sessions_state.lock().await.remove(&session_id);
    }

    Ok(ExecuteCommandResult {
        session_id,
        pid,
        initial_output: "Output is streamed via events. Listen to `terminal_output_<session_id>`.".to_string(),
        timed_out: timed_out_flag,
        exit_code: final_exit_code,
        message,
    })
}

#[tauri::command(async)]
#[instrument(skip(audit_logger_state, sessions_state, params), fields(session_id = %params.session_id))]
pub async fn force_terminate_session_command(
    audit_logger_state: State<'_, Arc<crate::utils::audit_logger::AuditLogger>>,
    sessions_state: State<'_, ActiveSessionsMap>,
    params: ForceTerminateParams,
) -> Result<ForceTerminateResult, AppError> {
    audit_log(&audit_logger_state, "force_terminate_session", &serde_json::to_value(params)?).await;

    let session_id = params.session_id;
    let session_arc_opt = sessions_state.lock().await.get(&session_id).cloned();

    if let Some(session_arc) = session_arc_opt {
        let mut child_guard = session_arc.process_child.lock().await;
        match child_guard.kill() {
            Ok(_) => {
                info!(sid = %session_id, pid = ?session_arc.pid, "Termination signal sent to process");
                *session_arc.exit_code.lock().await = Some(-9); // Mark as killed
                // The monitoring task will handle removal from sessions_state upon Terminated/Error/None event
                Ok(ForceTerminateResult {
                    session_id: session_id.clone(),
                    success: true,
                    message: "Termination signal sent.".to_string(),
                })
            }
            Err(e) => {
                warn!(sid = %session_id, pid = ?session_arc.pid, error = %e, "Failed to send kill signal");
                // If already terminated, exit_code might be set. If not, mark as error.
                if session_arc.exit_code.lock().await.is_none() {
                     *session_arc.exit_code.lock().await = Some(-10); // Indicate kill error
                }
                Ok(ForceTerminateResult {
                    session_id: session_id.clone(),
                    success: false,
                    message: format!("Failed to send kill signal: {}", e),
                })
            }
        }
    } else {
        Err(AppError::SessionNotFound(session_id))
    }
}

#[tauri::command(async)]
#[instrument(skip(audit_logger_state, sessions_state))]
pub async fn list_sessions_command(
    audit_logger_state: State<'_, Arc<crate::utils::audit_logger::AuditLogger>>,
    sessions_state: State<'_, ActiveSessionsMap>,
) -> Result<Vec<SessionInfo>, AppError> {
    audit_log(&audit_logger_state, "list_sessions", &serde_json::Value::Null).await;
    let sessions_map_guard = sessions_state.lock().await;
    let mut result_infos = Vec::new();
    let now_system = std::time::SystemTime::now();

    for (id, session_arc) in sessions_map_guard.iter() {
        let exit_code_guard = session_arc.exit_code.lock().await;
        let is_running = exit_code_guard.is_none();

        let runtime_duration = now_system.duration_since(session_arc.start_time_system)
            .unwrap_or_default(); // Handle potential time error gracefully

        result_infos.push(SessionInfo {
            session_id: id.clone(),
            command: session_arc.command_str.clone(),
            pid: session_arc.pid,
            is_running,
            start_time_iso: chrono::DateTime::<Utc>::from(session_arc.start_time_system).to_rfc3339(),
            runtime_ms: runtime_duration.as_millis(),
        });
    }
    Ok(result_infos)
}


#[tauri::command(async)]
#[instrument(skip(audit_logger_state, sessions_state, params), fields(session_id = %params.session_id))]
pub async fn read_session_output_status_command(
    audit_logger_state: State<'_, Arc<crate::utils::audit_logger::AuditLogger>>,
    sessions_state: State<'_, ActiveSessionsMap>,
    params: ReadOutputStatusParams,
) -> Result<ReadOutputStatusResult, AppError> {
    audit_log(&audit_logger_state, "read_session_output_status", &serde_json::to_value(params)?).await;
    let session_id = params.session_id;
    let session_arc_opt = sessions_state.lock().await.get(&session_id).cloned();

    if let Some(session_arc) = session_arc_opt {
        let exit_code = *session_arc.exit_code.lock().await;
        let is_running = exit_code.is_none();
        Ok(ReadOutputStatusResult {
            session_id: session_id.clone(),
            is_running,
            exit_code,
            message: "Session status retrieved. All output is streamed via events.".to_string(),
        })
    } else {
        Ok(ReadOutputStatusResult { // Return a specific status if not found, rather than erroring
            session_id: session_id.clone(),
            is_running: false,
            exit_code: None, // Or a specific code like -404
            message: "Session not found or already terminated.".to_string(),
        })
    }
}