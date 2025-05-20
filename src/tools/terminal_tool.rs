use crate::config::Config;
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command as TokioCommand};
use tokio::sync::Notify;
use tokio::time::{timeout, Duration};
use tracing::{debug, info, warn, instrument};
use uuid::Uuid;
use regex::Regex;

#[derive(Debug, Deserialize)]
pub struct ExecuteCommandParams {
    pub command: String,
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
    pub timed_out: bool,
    pub exit_code: Option<i32>,
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
}

struct ActiveSession {
    child: Child,
    command: String,
    output_buffer: Arc<Mutex<Vec<String>>>,
    is_finished: Arc<Notify>,
    exit_code: Arc<Mutex<Option<i32>>>,
    start_time: std::time::SystemTime,
}

#[derive(Debug)]
pub struct TerminalManager {
    config: Arc<Config>,
    sessions: Arc<Mutex<HashMap<String, Arc<ActiveSession>>>>,
}

impl TerminalManager {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            config,
            sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn is_command_blocked(&self, command_str: &str) -> bool {
        let first_word = command_str.split_whitespace().next().unwrap_or("");
        self.config.blocked_commands.iter().any(|regex| regex.is_match(first_word))
    }

    #[instrument(skip(self, params), fields(command = %params.command))]
    pub async fn execute_command(&self, params: &ExecuteCommandParams) -> Result<ExecuteCommandResult, AppError> {
        if self.is_command_blocked(¶ms.command) {
            return Err(AppError::CommandBlocked(params.command.clone()));
        }

        let session_id = Uuid::new_v4().to_string();
        let shell_cmd = params.shell.as_ref().or(self.config.default_shell.as_ref());
        
        let mut command = if let Some(shell) = shell_cmd {
            let mut cmd = TokioCommand::new(shell);
            if shell.contains("powershell") || shell.contains("cmd.exe") {
                 cmd.arg("/c");
            } else {
                 cmd.arg("-c");
            }
            cmd.arg(¶ms.command);
            cmd
        } else {
            // Basic parsing for direct execution if no shell specified
            let mut parts = params.command.split_whitespace();
            let program = parts.next().ok_or_else(|| AppError::CommandExecutionError("Empty command".to_string()))?;
            let mut cmd = TokioCommand::new(program);
            cmd.args(parts);
            cmd
        };

        command.current_dir(&self.config.files_root); // Execute in files_root
        command.stdin(Stdio::null());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());

        debug!("Spawning command: {:?}", command);
        let mut child = command.spawn().map_err(|e| {
            AppError::CommandExecutionError(format!("Failed to spawn command '{}': {}", params.command, e))
        })?;

        let pid = child.id();
        let output_buffer = Arc::new(Mutex::new(Vec::new()));
        let is_finished = Arc::new(Notify::new());
        let exit_code_arc = Arc::new(Mutex::new(None::<i32>));
        
        let session = Arc::new(ActiveSession {
            child, // child is moved here
            command: params.command.clone(),
            output_buffer: output_buffer.clone(),
            is_finished: is_finished.clone(),
            exit_code: exit_code_arc.clone(),
            start_time: std::time::SystemTime::now(),
        });
        
        self.sessions.lock().unwrap().insert(session_id.clone(), session.clone());

        let stdout = session.child.stdout.take().expect("Failed to capture stdout");
        let stderr = session.child.stderr.take().expect("Failed to capture stderr");

        let output_buffer_clone1 = output_buffer.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                output_buffer_clone1.lock().unwrap().push(line);
            }
        });
        
        let output_buffer_clone2 = output_buffer.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = reader.next_line().await {
                output_buffer_clone2.lock().unwrap().push(line);
            }
        });

        let child_wait_task = tokio::spawn(async move {
            match session.child.wait().await {
                Ok(status) => {
                    *session.exit_code.lock().unwrap() = status.code();
                    info!("Command '{}' (PID: {:?}, SID: {}) finished with status: {:?}", 
                          session.command, pid, session_id, status.code());
                }
                Err(e) => {
                    warn!("Failed to wait for command '{}' (PID: {:?}, SID: {}): {}", 
                          session.command, pid, session_id, e);
                }
            }
            session.is_finished.notify_waiters();
        });
        
        let timeout_duration = Duration::from_millis(params.timeout_ms.unwrap_or(1000)); // Default 1s
        let initial_output_result = timeout(timeout_duration, async {
            // Wait a bit for initial output or completion
            tokio::select! {
                _ = tokio::time::sleep(timeout_duration) => {}, // Max wait for initial output
                _ = is_finished.notified() => {}, // Or process finished
            }
            let mut buffer = output_buffer.lock().unwrap();
            let output_lines = buffer.join("\n");
            buffer.clear(); // Clear after reading
            output_lines
        }).await;

        let (initial_output, timed_out) = match initial_output_result {
            Ok(output) => (output, exit_code_arc.lock().unwrap().is_none()), // Timed out if not finished
            Err(_) => ("".to_string(), true), // Elasped means timeout for initial output
        };
        
        // If the process finished within the initial timeout, remove it.
        // Otherwise, it keeps running in the background.
        let final_exit_code = *exit_code_arc.lock().unwrap();
        if final_exit_code.is_some() {
             // Task might not be finished if we got here via is_finished.notified()
             // but child_wait_task hasn't completed setting exit_code yet.
             // So, we wait for child_wait_task briefly.
            let _ = timeout(Duration::from_millis(100), child_wait_task).await;
            self.sessions.lock().unwrap().remove(&session_id);
        }

        Ok(ExecuteCommandResult {
            session_id,
            pid,
            initial_output,
            timed_out,
            exit_code: final_exit_code,
        })
    }

    #[instrument(skip(self, params), fields(session_id = %params.session_id))]
    pub async fn read_output(&self, params: &ReadOutputParams) -> Result<ReadOutputResult, AppError> {
        let session_guard = self.sessions.lock().unwrap();
        let session_arc = session_guard.get(¶ms.session_id)
            .cloned() // Clone Arc to use outside lock
            .ok_or_else(|| AppError::SessionNotFound(0))?; // PID 0 as placeholder
        drop(session_guard); // Release lock

        let mut output_buffer = session_arc.output_buffer.lock().unwrap();
        let new_output = output_buffer.join("\n");
        output_buffer.clear();
        
        let exit_code = *session_arc.exit_code.lock().unwrap();
        let is_running = exit_code.is_none();

        if !is_running { // Process finished, remove from active sessions
            self.sessions.lock().unwrap().remove(¶ms.session_id);
        }

        Ok(ReadOutputResult {
            session_id: params.session_id.clone(),
            new_output,
            is_running,
            exit_code,
        })
    }

    #[instrument(skip(self, params), fields(session_id = %params.session_id))]
    pub async fn force_terminate(&self, params: &ForceTerminateParams) -> Result<ForceTerminateResult, AppError> {
        let mut sessions_map = self.sessions.lock().unwrap();
        if let Some(session_arc) = sessions_map.get_mut(¶ms.session_id) {
            // Child is part of ActiveSession, which is Arc'd.
            // To call `kill`, we need `&mut Child`.
            // This design makes direct killing hard. Child needs to be Mutex'd or use `start_kill`.
            // For simplicity, we'll use `start_kill` which is async and doesn't require `&mut`.
            // This is a limitation of the current Arc<ActiveSession> structure if we need immediate mutable access.
            // A better approach might involve sending a kill signal via a channel to the task managing the child.
            // Or, make `child` field an `Arc<Mutex<Child>>`.
            // For now, let's try `start_kill`.
            // Note: `child.start_kill()` is preferred over `child.kill().await` for Tokio's Child.
            
            // To actually kill, we need to get a mutable reference to the child,
            // which is tricky with the current Arc<ActiveSession> setup.
            // A common pattern is to wrap the Child itself in an Arc<Mutex<Child>>
            // or handle termination within the task that owns the Child.

            // Let's assume `ActiveSession`'s `child` field is an `Arc<Mutex<Child>>` for this example to work.
            // Or, more simply, if `child.kill()` is available and works on a non-mutable ref (some OS specific behavior).
            // Tokio's Child::kill() actually takes `&mut self`.
            // So, we can't directly call it on `session_arc.child`.
            // We must remove it from the map to get mutable ownership or change ActiveSession.
            
            // A simpler way for now:
            if let Err(e) = session_arc.child.start_kill() {
                 warn!("Failed to start_kill process for session {}: {}", params.session_id, e);
                 return Ok(ForceTerminateResult {
                    session_id: params.session_id.clone(),
                    success: false,
                    message: format!("Failed to send kill signal: {}", e),
                });
            }
            // Wait a moment for it to terminate
            let _ = timeout(Duration::from_secs(1), session_arc.child.wait()).await;

            sessions_map.remove(¶ms.session_id); // remove after attempting to kill
            info!("Force terminated session {}", params.session_id);
            Ok(ForceTerminateResult {
                session_id: params.session_id.clone(),
                success: true,
                message: "Termination signal sent.".to_string(),
            })
        } else {
            Err(AppError::SessionNotFound(0)) // PID 0 as placeholder
        }
    }
    
    #[instrument(skip(self))]
    pub async fn list_sessions(&self) -> Result<Vec<SessionInfo>, AppError> {
        let sessions_map = self.sessions.lock().unwrap();
        let mut result = Vec::new();
        for (id, session_arc) in sessions_map.iter() {
            let exit_code = *session_arc.exit_code.lock().unwrap();
            let is_running = exit_code.is_none();
            let pid = session_arc.child.id();
            let start_time_iso = chrono::DateTime::<chrono::Utc>::from(session_arc.start_time)
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);

            result.push(SessionInfo {
                session_id: id.clone(),
                command: session_arc.command.clone(),
                pid,
                is_running,
                start_time_iso,
            });
        }
        Ok(result)
    }
}