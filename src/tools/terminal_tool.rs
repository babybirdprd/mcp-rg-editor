use crate::config::Config;
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Stdio as StdProcessStdio;
use std::sync::{Arc, Mutex as StdMutex}; // Using std::sync::Mutex for cross-thread sharing
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command as TokioCommand};
use tokio::sync::Notify;
use tokio::time::{timeout, Duration};
use tracing::{debug, info, warn, instrument};
use uuid::Uuid;
use regex::Regex; // Already imported

#[derive(Debug, Deserialize)]
pub struct ExecuteCommandParams {
    pub command: String,
    #[serde(rename = "timeout_ms")]
    pub timeout_ms: Option<u64>,
    pub shell: Option<String>, // e.g. "bash", "powershell"
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
    pub timed_out: bool, // True if command exceeded initial timeout_ms and is running in background
    pub exit_code: Option<i32>, // Present if command finished within timeout_ms
    pub message: String, // General status message
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
    pub start_time_iso: String,
    pub runtime_ms: u128,
}

struct ActiveSession {
    child: Arc<StdMutex<Option<Child>>>, // Child needs to be mutable for kill, Option for take
    command: String,
    output_buffer: Arc<StdMutex<Vec<String>>>, // StdMutex for Vec
    is_finished_notify: Arc<Notify>, // Tokio's Notify
    exit_code: Arc<StdMutex<Option<i32>>>, // StdMutex for Option<i32>
    start_time: std::time::Instant,
    session_id: String, // Store session_id for logging within the task
    pid: Option<u32>, // Store PID for logging
}

#[derive(Debug)]
pub struct TerminalManager {
    config: Arc<Config>,
    sessions: Arc<StdMutex<HashMap<String, Arc<ActiveSession>>>>,
}

impl TerminalManager {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            config,
            sessions: Arc::new(StdMutex::new(HashMap::new())),
        }
    }

    fn is_command_blocked(&self, command_str: &str) -> bool {
        // Extract the base command (first word, ignoring leading env vars)
        let effective_command = command_str
            .trim_start()
            .split_whitespace()
            .find(|s| !s.contains('=')) // Skip KEY=value parts
            .unwrap_or("");

        self.config.blocked_commands.iter().any(|regex| {
            regex.is_match(effective_command)
        })
    }

    #[instrument(skip(self, params), fields(command = %params.command))]
    pub async fn execute_command(&self, params: &ExecuteCommandParams) -> Result<ExecuteCommandResult, AppError> {
        if self.is_command_blocked(¶ms.command) {
            warn!(command = %params.command, "Command execution blocked");
            return Err(AppError::CommandBlocked(params.command.clone()));
        }

        let session_id = Uuid::new_v4().to_string();
        let shell_to_use = params.shell.as_ref().or(self.config.default_shell.as_ref());
        
        let mut command_process = if let Some(shell_path) = shell_to_use {
            let mut cmd = TokioCommand::new(shell_path);
            if shell_path.contains("powershell") || shell_path.contains("cmd.exe") {
                 cmd.arg("-Command"); // Or /c for cmd.exe
            } else { // Assuming sh-like shell
                 cmd.arg("-c");
            }
            cmd.arg(¶ms.command.clone()); // Pass the whole command string to the shell
            cmd
        } else {
            // Basic parsing for direct execution if no shell specified
            let mut parts = params.command.split_whitespace();
            let program = parts.next().ok_or_else(|| AppError::CommandExecutionError("Empty command".to_string()))?;
            let mut cmd = TokioCommand::new(program);
            cmd.args(parts);
            cmd
        };

        command_process.current_dir(&self.config.files_root); 
        command_process.stdin(StdProcessStdio::null());
        command_process.stdout(StdProcessStdio::piped());
        command_process.stderr(StdProcessStdio::piped());

        debug!(shell = ?shell_to_use, command = %params.command, "Spawning command");
        
        let child_instance = command_process.spawn().map_err(|e| {
            error!(error = %e, command = %params.command, "Failed to spawn command");
            AppError::CommandExecutionError(format!("Failed to spawn command '{}': {}", params.command, e))
        })?;
        
        let pid = child_instance.id();
        let command_str_clone = params.command.clone(); // Clone for the task

        let active_session = Arc::new(ActiveSession {
            child: Arc::new(StdMutex::new(Some(child_instance))), // Wrap child in Option and Mutex
            command: command_str_clone.clone(),
            output_buffer: Arc::new(StdMutex::new(Vec::new())),
            is_finished_notify: Arc::new(Notify::new()),
            exit_code: Arc::new(StdMutex::new(None::<i32>)),
            start_time: std::time::Instant::now(),
            session_id: session_id.clone(),
            pid,
        });
        
        self.sessions.lock().unwrap().insert(session_id.clone(), active_session.clone());
        
        // Spawn a task to manage the child process's lifecycle and output
        let session_clone_for_task = active_session.clone();
        tokio::spawn(async move {
            let mut child_opt = session_clone_for_task.child.lock().unwrap().take(); // Take ownership of child
            if let Some(mut child_process) = child_opt {
                let stdout = child_process.stdout.take().expect("Failed to capture stdout from child");
                let stderr = child_process.stderr.take().expect("Failed to capture stderr from child");

                let stdout_buffer_clone = session_clone_for_task.output_buffer.clone();
                let stderr_buffer_clone = session_clone_for_task.output_buffer.clone(); // Log both to same buffer

                let stdout_task = tokio::spawn(async move {
                    let mut reader = BufReader::new(stdout).lines();
                    while let Ok(Some(line)) = reader.next_line().await {
                        stdout_buffer_clone.lock().unwrap().push(format!("[stdout] {}", line));
                    }
                });

                let stderr_task = tokio::spawn(async move {
                    let mut reader = BufReader::new(stderr).lines();
                    while let Ok(Some(line)) = reader.next_line().await {
                        stderr_buffer_clone.lock().unwrap().push(format!("[stderr] {}", line));
                    }
                });
                
                let status_result = child_process.wait().await;
                
                // Ensure output tasks complete
                let _ = tokio::join!(stdout_task, stderr_task);

                match status_result {
                    Ok(status) => {
                        *session_clone_for_task.exit_code.lock().unwrap() = status.code();
                        info!(command = %session_clone_for_task.command, pid = ?session_clone_for_task.pid, sid = %session_clone_for_task.session_id, exit_code = ?status.code(), "Command finished");
                    }
                    Err(e) => {
                        warn!(command = %session_clone_for_task.command, pid = ?session_clone_for_task.pid, sid = %session_clone_for_task.session_id, error = %e, "Failed to wait for command");
                        // Store a generic error code if wait fails
                        *session_clone_for_task.exit_code.lock().unwrap() = Some(-1); 
                    }
                }
                session_clone_for_task.is_finished_notify.notify_waiters();
            } else {
                 warn!(sid=%session_clone_for_task.session_id, "Child process already taken or None in monitoring task");
            }
        });
        
        let timeout_duration = Duration::from_millis(params.timeout_ms.unwrap_or(1000));
        
        let initial_output_string = match timeout(timeout_duration, active_session.is_finished_notify.notified()).await {
            Ok(_) => { // Process finished within timeout
                let mut buffer = active_session.output_buffer.lock().unwrap();
                let output = buffer.join("\n");
                buffer.clear();
                output
            }
            Err(_) => { // Timeout elapsed, process might still be running
                let mut buffer = active_session.output_buffer.lock().unwrap();
                let output = buffer.join("\n");
                buffer.clear();
                output
            }
        };
        
        let final_exit_code = *active_session.exit_code.lock().unwrap();
        let timed_out = final_exit_code.is_none(); // If no exit code, it timed out for initial output

        let message = if timed_out {
            format!("Command started with PID {:?}, Session ID {}. Running in background.", pid, session_id)
        } else {
            format!("Command finished with PID {:?}, Session ID {}. Exit code: {:?}.", pid, session_id, final_exit_code)
        };
        
        if !timed_out { // If finished, remove from active sessions
            self.sessions.lock().unwrap().remove(&session_id);
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
        let session_arc = { // Scope for the lock
            let sessions_map = self.sessions.lock().unwrap();
            sessions_map.get(¶ms.session_id).cloned() 
        };

        if let Some(session_arc_unwrapped) = session_arc {
            let mut output_buffer_guard = session_arc_unwrapped.output_buffer.lock().unwrap();
            let new_output = output_buffer_guard.join("\n");
            output_buffer_guard.clear();
            
            let exit_code_guard = session_arc_unwrapped.exit_code.lock().unwrap();
            let exit_code = *exit_code_guard;
            let is_running = exit_code.is_none();

            if !is_running { 
                self.sessions.lock().unwrap().remove(¶ms.session_id);
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
        let session_arc = {
            let mut sessions_map = self.sessions.lock().unwrap();
            sessions_map.remove(¶ms.session_id) // Remove to get mutable ownership effectively
        };

        if let Some(session_arc_unwrapped) = session_arc {
            let mut child_guard = session_arc_unwrapped.child.lock().unwrap();
            if let Some(child_process) = child_guard.as_mut() {
                match child_process.start_kill() {
                    Ok(_) => {
                        info!(sid = %params.session_id, pid = ?session_arc_unwrapped.pid, "Termination signal sent to process");
                        // Optionally wait a bit for the process to die
                        let _ = timeout(Duration::from_millis(500), child_process.wait()).await;
                        Ok(ForceTerminateResult {
                            session_id: params.session_id.clone(),
                            success: true,
                            message: "Termination signal sent.".to_string(),
                        })
                    }
                    Err(e) => {
                        warn!(sid = %params.session_id, pid = ?session_arc_unwrapped.pid, error = %e, "Failed to send kill signal");
                        // Put it back if kill failed, though it's in a weird state
                        self.sessions.lock().unwrap().insert(params.session_id.clone(), session_arc_unwrapped);
                        Ok(ForceTerminateResult {
                            session_id: params.session_id.clone(),
                            success: false,
                            message: format!("Failed to send kill signal: {}", e),
                        })
                    }
                }
            } else {
                // Child already taken/None, means it was likely already processed/terminated
                 Ok(ForceTerminateResult {
                    session_id: params.session_id.clone(),
                    success: false,
                    message: "Process already terminated or not found in session.".to_string(),
                })
            }
        } else {
            Err(AppError::SessionNotFound(params.session_id.clone()))
        }
    }
    
    #[instrument(skip(self))]
    pub async fn list_sessions(&self) -> Result<Vec<SessionInfo>, AppError> {
        let sessions_map = self.sessions.lock().unwrap();
        let mut result_infos = Vec::new();
        let now_instant = std::time::Instant::now();

        for (id, session_arc) in sessions_map.iter() {
            let exit_code_guard = session_arc.exit_code.lock().unwrap();
            let is_running = exit_code_guard.is_none();
            
            let runtime_ms = now_instant.duration_since(session_arc.start_time).as_millis();
            let start_time_system = std::time::SystemTime::UNIX_EPOCH + Duration::from_nanos(
                session_arc.start_time.duration_since(std::time::Instant::now() - std::time::Duration::from_millis(runtime_ms as u64)).as_nanos() as u64
            ); // Approximate system time from instant

            result_infos.push(SessionInfo {
                session_id: id.clone(),
                command: session_arc.command.clone(),
                pid: session_arc.pid,
                is_running,
                start_time_iso: chrono::DateTime::<chrono::Utc>::from(start_time_system).to_rfc3339(),
                runtime_ms,
            });
        }
        Ok(result_infos)
    }
}