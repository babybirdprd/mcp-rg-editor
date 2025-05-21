// FILE: src/tools/process_tool.rs
use crate::config::Config;
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock as StdRwLock};
use sysinfo::{Pid, Signal, Uid, System, ProcessRefreshKind, RefreshKind};
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
    uid_opt.map(|uid| format!("{}", uid))
}

impl ProcessManager {
    pub fn new(config: Arc<StdRwLock<Config>>) -> Self {
        let mut sys = System::new_with_specifics(RefreshKind::new().with_processes(ProcessRefreshKind::everything()));
        sys.refresh_processes_specifics(ProcessRefreshKind::everything());
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
        sys_guard.refresh_processes_specifics(ProcessRefreshKind::everything());
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
        
        let pid_to_kill = Pid::from(params.pid);
        debug!(target_pid = %pid_to_kill, "Attempting to kill process");

        // Refresh only the specific process information
        sys_guard.refresh_process_specifics(pid_to_kill, ProcessRefreshKind::everything());

        let process_name_for_message = match sys_guard.process(pid_to_kill) {
            Some(p) => p.name().to_string(),
            None => {
                return Ok(KillProcessResult {
                    success: false,
                    message: format!("Process with PID {} not found.", params.pid),
                });
            }
        };

        // Try SIGTERM
        // Re-fetch process before attempting to kill, as its state might have changed
        if let Some(p_term) = sys_guard.process(pid_to_kill) {
            if p_term.kill_with(Signal::Term).unwrap_or(false) {
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                sys_guard.refresh_process_specifics(pid_to_kill, ProcessRefreshKind::everything()); // Refresh after signal
                if sys_guard.process(pid_to_kill).is_none() {
                    debug!(pid = %pid_to_kill, "Process terminated with SIGTERM");
                    return Ok(KillProcessResult {
                        success: true,
                        message: format!("Process {} ({}) terminated successfully with SIGTERM.", params.pid, process_name_for_message),
                    });
                }
                debug!(pid = %pid_to_kill, "Process still alive after SIGTERM attempt and refresh.");
            } else {
                 debug!(pid = %pid_to_kill, "Sending SIGTERM failed or process already gone.");
            }
        } // p_term is dropped here, releasing immutable borrow from sys_guard.process()

        // Try SIGKILL if still present or SIGTERM failed
        sys_guard.refresh_process_specifics(pid_to_kill, ProcessRefreshKind::everything()); // Refresh again before SIGKILL
        if let Some(p_kill) = sys_guard.process(pid_to_kill) { // New immutable borrow
            if p_kill.kill_with(Signal::Kill).unwrap_or(false) {
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                sys_guard.refresh_process_specifics(pid_to_kill, ProcessRefreshKind::everything()); // Refresh after signal
                if sys_guard.process(pid_to_kill).is_none() {
                    debug!(pid = %pid_to_kill, "Process terminated with SIGKILL");
                    return Ok(KillProcessResult {
                        success: true,
                        message: format!("Process {} ({}) terminated successfully with SIGKILL.", params.pid, process_name_for_message),
                    });
                } else {
                    warn!(pid = %pid_to_kill, "Process still running after SIGKILL");
                    return Ok(KillProcessResult {
                        success: false,
                        message: format!("Sent SIGKILL to process {} ({}), but it might still be running. OS may prevent killing certain processes.", params.pid, process_name_for_message),
                    });
                }
            } else {
                warn!(pid = %pid_to_kill, "Failed to send SIGKILL");
                // Process might have been terminated by SIGTERM and refresh was slow, or it's truly unkillable by this user
                sys_guard.refresh_process_specifics(pid_to_kill, ProcessRefreshKind::everything());
                if sys_guard.process(pid_to_kill).is_none() {
                     return Ok(KillProcessResult {
                        success: true,
                        message: format!("Process {} ({}) no longer found after failed SIGKILL attempt, likely terminated by previous SIGTERM or exited.", params.pid, process_name_for_message),
                    });
                }
                return Ok(KillProcessResult {
                    success: false,
                    message: format!("Failed to send SIGKILL to process {} ({}). It might require higher privileges or be a system process.", params.pid, process_name_for_message),
                });
            }
        } else { // Process not found before SIGKILL attempt
            debug!(pid = %pid_to_kill, "Process not found before SIGKILL attempt, assuming terminated by SIGTERM or exited.");
            return Ok(KillProcessResult {
                success: true, // If it's gone, that's a success from a killing perspective
                message: format!("Process {} ({}) no longer found, likely terminated.", params.pid, process_name_for_message),
            });
        }
    }
}