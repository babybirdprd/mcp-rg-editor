use crate::config::Config;
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use sysinfo::{Pid, ProcessExt, System, SystemExt, Signal};
use tracing::{instrument, debug, warn};
use std::sync::Mutex as StdMutex; // Using std::sync::Mutex as sysinfo is sync

#[derive(Debug, Deserialize)]
pub struct KillProcessParams {
    pub pid: usize, // sysinfo uses usize for PIDs
}

#[derive(Debug, Serialize)]
pub struct ProcessInfo {
    pid: String, 
    name: String,
    cpu_usage: f32,
    memory_mb: u64,
    command: String,
    status: String, // e.g. Running, Sleeping
    user: Option<String>,
    start_time_epoch_secs: u64,
}

#[derive(Debug, Serialize)]
pub struct KillProcessResult {
    pub success: bool,
    pub message: String,
}

#[derive(Debug)]
pub struct ProcessManager {
    _config: Arc<Config>, // Keep config for future use (e.g. filtering by allowed commands)
    system: Arc<StdMutex<System>>, // Mutex for interior mutability of System
}

impl ProcessManager {
    pub fn new(config: Arc<Config>) -> Self {
        let mut sys = System::new_all();
        sys.refresh_all(); 
        Self {
            _config: config,
            system: Arc::new(StdMutex::new(sys)),
        }
    }

    #[instrument(skip(self))]
    pub async fn list_processes(&self) -> Result<Vec<ProcessInfo>, AppError> {
        let mut sys_guard = self.system.lock().map_err(|e| {
            AppError::ProcessError(format!("Failed to lock system mutex for list_processes: {}", e))
        })?;
        sys_guard.refresh_processes(); 
        debug!("Listing system processes. Found {} processes.", sys_guard.processes().len());

        let mut processes_info = Vec::new();
        for (pid_obj, process) in sys_guard.processes() {
            processes_info.push(ProcessInfo {
                pid: pid_obj.to_string(),
                name: process.name().to_string(),
                cpu_usage: process.cpu_usage(),
                memory_mb: process.memory() / (1024 * 1024), // Bytes to MB
                command: process.cmd().join(" "),
                status: process.status().to_string(),
                user: process.user_id().map(|uid| format!("{:?}", uid)), // May not always be available or meaningful
                start_time_epoch_secs: process.start_time(),
            });
        }
        Ok(processes_info)
    }

    #[instrument(skip(self, params), fields(pid = %params.pid))]
    pub async fn kill_process(&self, params: &KillProcessParams) -> Result<KillProcessResult, AppError> {
        let mut sys_guard = self.system.lock().map_err(|e| {
            AppError::ProcessError(format!("Failed to lock system mutex for kill_process: {}", e))
        })?;
        sys_guard.refresh_processes();
        
        let pid_to_kill = Pid::from(params.pid);
        debug!(target_pid = %pid_to_kill, "Attempting to kill process");

        if let Some(process) = sys_guard.process(pid_to_kill) {
            // Try SIGTERM first, then SIGKILL if needed (common practice)
            if process.kill_with(Signal::Term).unwrap_or(false) {
                // Wait a moment to see if it terminates
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                sys_guard.refresh_process(pid_to_kill); // Refresh specific process
                if sys_guard.process(pid_to_kill).is_none() {
                     debug!(pid = %pid_to_kill, "Process terminated with SIGTERM");
                    return Ok(KillProcessResult {
                        success: true,
                        message: format!("Process {} ({}) terminated successfully with SIGTERM.", params.pid, process.name()),
                    });
                }
            }
            
            // If still running, try SIGKILL
            warn!(pid = %pid_to_kill, "Process did not terminate with SIGTERM, trying SIGKILL");
            if process.kill_with(Signal::Kill).unwrap_or(false) {
                 // SIGKILL is usually immediate, but let's refresh to confirm
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                sys_guard.refresh_process(pid_to_kill);
                if sys_guard.process(pid_to_kill).is_none() {
                    debug!(pid = %pid_to_kill, "Process terminated with SIGKILL");
                    Ok(KillProcessResult {
                        success: true,
                        message: format!("Process {} ({}) terminated successfully with SIGKILL.", params.pid, process.name()),
                    })
                } else {
                    warn!(pid = %pid_to_kill, "Process still running after SIGKILL");
                     Ok(KillProcessResult {
                        success: false,
                        message: format!("Sent SIGKILL to process {} ({}), but it might still be running. OS may prevent killing certain processes.", params.pid, process.name()),
                    })
                }
            } else {
                warn!(pid = %pid_to_kill, "Failed to send SIGKILL");
                Ok(KillProcessResult {
                    success: false,
                    message: format!("Failed to send termination signal to process {} ({}). It might require higher privileges or be a system process.", params.pid, process.name()),
                })
            }
        } else {
            warn!(pid = %pid_to_kill, "Process not found");
            Ok(KillProcessResult {
                success: false,
                message: format!("Process with PID {} not found.", params.pid),
            })
        }
    }
}