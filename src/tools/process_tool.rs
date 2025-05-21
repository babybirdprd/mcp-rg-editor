// FILE: src/tools/process_tool.rs
use crate::config::Config;
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock as StdRwLock};
use sysinfo::{Pid, ProcessExt, System, SystemExt, Signal, Uid}; // Removed Gid
use tracing::{instrument, debug, warn};
use std::sync::Mutex as StdMutexForSysinfo;

#[derive(Debug, Deserialize)]
pub struct KillProcessParams {
    pub pid: usize, 
}

#[derive(Debug, Serialize)]
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

#[derive(Debug)]
pub struct ProcessManager {
    _config: Arc<StdRwLock<Config>>,
    system: Arc<StdMutexForSysinfo<System>>,
}

fn format_uid(uid_opt: Option<&Uid>) -> Option<String> {
    uid_opt.map(|uid| format!("{:?}", uid))
}

impl ProcessManager {
    pub fn new(config: Arc<StdRwLock<Config>>) -> Self {
        let mut sys = System::new_all();
        sys.refresh_all(); 
        Self {
            _config: config,
            system: Arc::new(StdMutexForSysinfo::new(sys)),
        }
    }

    #[instrument(skip(self))]
    pub async fn list_processes(&self) -> Result<Vec<ProcessInfo>, AppError> {
        let mut sys_guard = self.system.lock().map_err(|e| {
            AppError::ProcessError(format!("Failed to lock system mutex for list_processes: {}", e))
        })?;
        sys_guard.refresh_processes(); 
        debug!("Listing system processes. Found {} processes.", sys_guard.processes().len());

        let processes_info = sys_guard.processes().iter().map(|(pid_obj, process)| {
            ProcessInfo {
                pid: pid_obj.as_u32().to_string(),
                name: process.name().to_string(),
                cpu_usage: process.cpu_usage(),
                memory_mb: process.memory() / (1024 * 1024), 
                command: process.cmd().join(" "),
                status: process.status().to_string(),
                user: format_uid(process.user_id()),
                start_time_epoch_secs: process.start_time(),
            }
        }).collect();
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
            if process.kill_with(Signal::Term).unwrap_or(false) {
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                sys_guard.refresh_process(pid_to_kill); 
                if sys_guard.process(pid_to_kill).is_none() { 
                     debug!(pid = %pid_to_kill, "Process terminated with SIGTERM");
                    return Ok(KillProcessResult {
                        success: true,
                        message: format!("Process {} ({}) terminated successfully with SIGTERM.", params.pid, process.name()),
                    });
                }
            }
            
            warn!(pid = %pid_to_kill, "Process did not terminate with SIGTERM, trying SIGKILL");
            if process.kill_with(Signal::Kill).unwrap_or(false) { // Use kill_with for sysinfo 0.30+
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