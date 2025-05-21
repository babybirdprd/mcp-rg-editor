use crate::config::Config;
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Stdio as StdProcessStdio;
use std::sync::{Arc, RwLock as StdRwLock};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command as TokioCommand};
use tokio::sync::{Mutex as TokioMutex, Notify}; // Use TokioMutex
use tokio::time::{timeout, Duration};
use tracing::{debug, info, warn, instrument, error};
use uuid::Uuid;
use chrono::Utc; // For SessionInfo start_time_iso

#[derive(Debug, Deserialize)]
pub struct ExecuteCommandParams {
    pub command: String,
    #[serde(rename = "timeout_ms")]
    pub timeout_ms: Option<u64>,
    pub shell: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ReadOutputParams {
    pub session_id: String,
}

#[derive(Debug, Deserialize)]
pub struct ForceTerminateParams {
    pub session_id: String,
}

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
pub struct ReadOutputResult {
    pub session_id: String,
    pub new_output: String,
    pub is_running: bool,
    pub exit_code: Option<i32>,
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
    pub start_time_iso: String, // Changed to ISO string
    pub runtime_ms: u128,
}

#[derive(Debug)]
struct ActiveSession {
    child_mutex: Arc<TokioMutex<Option<Child>>>, // Changed to TokioMutex
    command: String,
    output_buffer: Arc<TokioMutex<Vec<String>>>, // Changed to TokioMutex
    is_finished_notify: Arc<Notify>,
    exit_code: Arc<TokioMutex<Option<i32>>>, // Changed to TokioMutex
    start_time: std::time::Instant,
    start_time_system: std::time::SystemTime, // Store SystemTime for accurate ISO
    session_id: String,
    pid: Option<u32>,
}

#[derive(Debug)]
pub struct TerminalManager {
    config: Arc<StdRwLock<Config>>,
    sessions: Arc<TokioMutex<HashMap<String, Arc<ActiveSession>>>>, // Changed to TokioMutex
}

impl TerminalManager {
    pub fn new(config: Arc<StdRwLock<Config>>) -> Self {
        Self {
            config,
            sessions: Arc::new(TokioMutex::new(HashMap::new())),
        }
    }

    fn is_command_blocked(&self, command_str: &str, config_guard: &Config) -> bool {
        let effective_command = command_str
            .trim_start()
            .split_whitespace()
            .find(|s| !s.contains('=')) // Skip leading VAR=val assignments
            .unwrap_or("");

        config_guard.blocked_commands.iter().any(|regex| {
            regex.is_match(effective_command)
        })
    }

    #[instrument(skip(self, params), fields(command = %params.command))]
    pub async fn execute_command(&self, params: &ExecuteCommandParams) -> Result<ExecuteCommandResult, AppError> {
        let config_guard = self.config.read().map_err(|e| AppError::ConfigError(anyhow::anyhow!("Config lock poisoned: {}", e)))?;
        if self.is_command_blocked(&params.command, &config_guard) {
            warn!(command = %params.command, "Command execution blocked");
            return Err(AppError::CommandBlocked(params.command.clone()));
        }

        let session_id = Uuid::new_v4().to_string();
        let shell_to_use = params.shell.as_ref().or(config_guard.default_shell.as_ref());
        
        let mut command_process = if let Some(shell_path) = shell_to_use {
            let mut cmd = TokioCommand::new(shell_path);
            if shell_path.contains("powershell") || shell_path.contains("cmd.exe") {
                 cmd.arg("-Command"); // Or /C for cmd.exe
            } else {
                 cmd.arg("-c");
            }
            cmd.arg(&params.command);
            cmd
        } else {
            // Basic parsing for direct command execution
            // This is a simplification; a proper shell parser would be more robust
            let mut parts = params.command.split_whitespace();
            let program = parts.next().ok_or_else(|| AppError::CommandExecutionError("Empty command".to_string()))?;
            let mut cmd = TokioCommand::new(program);
            cmd.args(parts);
            cmd
        };

        command_process.current_dir(&config_guard.files_root); 
        drop(config_guard);

        command_process.stdin(StdProcessStdio::null());
        command_process.stdout(StdProcessStdio::piped());
        command_process.stderr(StdProcessStdio::piped());

        debug!(shell = ?shell_to_use, command = %params.command, "Spawning command");
        
        let child_instance = command_process.spawn().map_err(|e| {
            error!(error = %e, command = %params.command, "Failed to spawn command");
            AppError::CommandExecutionError(format!("Failed to spawn command '{}': {}", params.command, e))
        })?;
        
        let pid = child_instance.id();
        let command_str_clone = params.command.clone();

        let active_session = Arc::new(ActiveSession {
            child_mutex: Arc::new(TokioMutex::new(Some(child_instance))),
            command: command_str_clone.clone(),
            output_buffer: Arc::new(TokioMutex::new(Vec::new())),
            is_finished_notify: Arc::new(Notify::new()),
            exit_code: Arc::new(TokioMutex::new(None::<i32>)),
            start_time: std::time::Instant::now(),
            start_time_system: std::time::SystemTime::now(),
            session_id: session_id.clone(),
            pid,
        });
        
        self.sessions.lock().await.insert(session_id.clone(), active_session.clone());
        
        let session_clone_for_task = active_session.clone();
        tokio::spawn(async move {
            let mut child_opt_guard = session_clone_for_task.child_mutex.lock().await;
            if let Some(mut child_process) = child_opt_guard.take() {
                drop(child_opt_guard); 

                let stdout = child_process.stdout.take().expect("Failed to capture stdout from child");
                let stderr = child_process.stderr.take().expect("Failed to capture stderr from child");

                let output_buffer_clone_stdout = session_clone_for_task.output_buffer.clone();
                let output_buffer_clone_stderr = session_clone_for_task.output_buffer.clone();

                let stdout_task = tokio::spawn(async move {
                    let mut reader = BufReader::new(stdout).lines();
                    while let Ok(Some(line)) = reader.next_line().await {
                        output_buffer_clone_stdout.lock().await.push(format!("[stdout] {}", line));
                    }
                });

                let stderr_task = tokio::spawn(async move {
                    let mut reader = BufReader::new(stderr).lines();
                    while let Ok(Some(line)) = reader.next_line().await {
                        output_buffer_clone_stderr.lock().await.push(format!("[stderr] {}", line));
                    }
                });
                
                let status_result = child_process.wait().await;
                
                let _ = tokio::join!(stdout_task, stderr_task); // Wait for I/O tasks to finish

                match status_result {
                    Ok(status) => {
                        *session_clone_for_task.exit_code.lock().await = status.code();
                        info!(command = %session_clone_for_task.command, pid = ?session_clone_for_task.pid, sid = %session_clone_for_task.session_id, exit_code = ?status.code(), "Command finished");
                    }
                    Err(e) => {
                        warn!(command = %session_clone_for_task.command, pid = ?session_clone_for_task.pid, sid = %session_clone_for_task.session_id, error = %e, "Failed to wait for command");
                        *session_clone_for_task.exit_code.lock().await = Some(-1); // Indicate error
                    }
                }
                session_clone_for_task.is_finished_notify.notify_waiters();
            } else {
                 warn!(sid=%session_clone_for_task.session_id, "Child process already taken or None in monitoring task");
                 if session_clone_for_task.exit_code.lock().await.is_none() {
                    *session_clone_for_task.exit_code.lock().await = Some(-2);
                 }
                 session_clone_for_task.is_finished_notify.notify_waiters();
            }
        });
        
        let timeout_duration = Duration::from_millis(params.timeout_ms.unwrap_or(1000));
        
        let initial_output_string = match timeout(timeout_duration, active_session.is_finished_notify.notified()).await {
            Ok(_) => { // Notified means finished within timeout
                let mut buffer = active_session.output_buffer.lock().await;
                let output = buffer.join("\n");
                buffer.clear();
                output
            }
            Err(_) => { // Timed out
                let mut buffer = active_session.output_buffer.lock().await;
                let output = buffer.join("\n");
                buffer.clear();
                output
            }
        };
        
        let final_exit_code = *active_session.exit_code.lock().await;
        let timed_out = final_exit_code.is_none(); // If no exit code, it timed out or is still running

        let message = if timed_out {
            format!("Command started with PID {:?}, Session ID {}. Running in background.", pid, session_id)
        } else {
            format!("Command finished with PID {:?}, Session ID {}. Exit code: {:?}.", pid, session_id, final_exit_code)
        };
        
        if !timed_out { // If finished (not timed out), remove from active sessions
            self.sessions.lock().await.remove(&session_id);
        }

        Ok(ExecuteCommandResult {
            session_id,
            pid,
            initial_output: initial_output_string,
            timed_out,
            exit_code: final_exit_code,
            message,
        })
    }

    #[instrument(skip(self, params), fields(session_id = %params.session_id))]
    pub async fn read_output(&self, params: &ReadOutputParams) -> Result<ReadOutputResult, AppError> {
        let session_arc = {
            let sessions_map_guard = self.sessions.lock().await;
            sessions_map_guard.get(&params.session_id).cloned()
        };

        if let Some(session_arc_unwrapped) = session_arc {
            let mut output_buffer_guard = session_arc_unwrapped.output_buffer.lock().await;
            let new_output = output_buffer_guard.join("\n");
            output_buffer_guard.clear();
            
            let exit_code_guard = session_arc_unwrapped.exit_code.lock().await;
            let exit_code = *exit_code_guard;
            let is_running = exit_code.is_none();

            if !is_running { 
                self.sessions.lock().await.remove(&params.session_id);
            }

            Ok(ReadOutputResult {
                session_id: params.session_id.clone(),
                new_output,
                is_running,
                exit_code,
            })
        } else {
            Err(AppError::SessionNotFound(params.session_id.clone()))
        }
    }

    #[instrument(skip(self, params), fields(session_id = %params.session_id))]
    pub async fn force_terminate(&self, params: &ForceTerminateParams) -> Result<ForceTerminateResult, AppError> {
        let session_arc_opt = {
            let mut sessions_map_guard = self.sessions.lock().await;
            sessions_map_guard.remove(&params.session_id)
        };

        if let Some(session_arc_unwrapped) = session_arc_opt {
            let mut child_guard = session_arc_unwrapped.child_mutex.lock().await;
            if let Some(child_process) = child_guard.as_mut() {
                match child_process.start_kill() {
                    Ok(_) => {
                        info!(sid = %params.session_id, pid = ?session_arc_unwrapped.pid, "Termination signal sent to process");
                        // Optionally wait a bit for the process to exit
                        let _ = timeout(Duration::from_millis(500), child_process.wait()).await;
                        *session_arc_unwrapped.exit_code.lock().await = Some(-9); // Mark as killed
                        session_arc_unwrapped.is_finished_notify.notify_waiters();
                        Ok(ForceTerminateResult {
                            session_id: params.session_id.clone(),
                            success: true,
                            message: "Termination signal sent.".to_string(),
                        })
                    }
                    Err(e) => {
                        warn!(sid = %params.session_id, pid = ?session_arc_unwrapped.pid, error = %e, "Failed to send kill signal");
                        // Reinsert if kill failed, as it's still technically active though we tried to kill
                        self.sessions.lock().await.insert(params.session_id.clone(), session_arc_unwrapped);
                        Ok(ForceTerminateResult {
                            session_id: params.session_id.clone(),
                            success: false,
                            message: format!("Failed to send kill signal: {}", e),
                        })
                    }
                }
            } else { // Child was already None
                 Ok(ForceTerminateResult {
                    session_id: params.session_id.clone(),
                    success: false, // Or true if "already terminated" is considered success
                    message: "Process already terminated or not found in session.".to_string(),
                })
            }
        } else {
            Err(AppError::SessionNotFound(params.session_id.clone()))
        }
    }
    
    #[instrument(skip(self))]
    pub async fn list_sessions(&self) -> Result<Vec<SessionInfo>, AppError> {
        let sessions_map_guard = self.sessions.lock().await;
        let mut result_infos = Vec::new();
        let now_instant = std::time::Instant::now();

        for (id, session_arc) in sessions_map_guard.iter() {
            let exit_code_guard = session_arc.exit_code.lock().await;
            let is_running = exit_code_guard.is_none();
            
            let runtime_ms = now_instant.duration_since(session_arc.start_time).as_millis();
            
            let start_time_iso = chrono::DateTime::<Utc>::from(session_arc.start_time_system).to_rfc3339();

            result_infos.push(SessionInfo {
                session_id: id.clone(),
                command: session_arc.command.clone(),
                pid: session_arc.pid,
                is_running,
                start_time_iso,
                runtime_ms,
            });
        }
        Ok(result_infos)
    }
}