use crate::config::Config;
use crate::error::AppError;
use crate::utils::audit_logger::audit_log;

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use sysinfo::{Pid, Signal, System, ProcessRefreshKind, RefreshKind, Uid};
use tauri::{AppHandle, Manager, State};
use tokio::sync::Mutex as TokioMutex; // For sysinfo state
use tracing::{debug, instrument, warn};


// --- Request Structs ---
#[derive(Debug, Deserialize, Serialize)] // Added Serialize for audit log
pub struct KillProcessParams {
    pub pid: usize, // sysinfo::Pid takes usize
}

// --- Response Structs ---
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

// --- Sysinfo State ---
// Storing System in Tauri state, wrapped in TokioMutex for async access
type SysinfoState = Arc<TokioMutex<System>>;

fn format_uid_internal(uid_opt: Option<&Uid>) -> Option<String> {
    uid_opt.map(|uid| format!("{}", uid)) // Uid itself implements Display
}


#[tauri::command(async)]
#[instrument(skip(app_handle, audit_logger_state, sysinfo_state))]
pub async fn list_processes_command(
    app_handle: AppHandle, // Keep for consistency, might be used for notifications
    audit_logger_state: State<'_, Arc<crate::utils::audit_logger::AuditLogger>>,
    sysinfo_state: State<'_, SysinfoState>,
) -> Result<Vec<ProcessInfo>, AppError> {
    audit_log(&audit_logger_state, "list_processes", &serde_json::Value::Null).await;

    let mut sys_guard = sysinfo_state.lock().await;
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
            user: format_uid_internal(process.user_id()),
            start_time_epoch_secs: process.start_time(),
        }
    }).collect();
    Ok(processes_info)
}


#[tauri::command(async)]
#[instrument(skip(app_handle, audit_logger_state, sysinfo_state, params), fields(pid = %params.pid))]
pub async fn kill_process_command(
    app_handle: AppHandle, // Keep for consistency
    audit_logger_state: State<'_, Arc<crate::utils::audit_logger::AuditLogger>>,
    sysinfo_state: State<'_, SysinfoState>,
    params: KillProcessParams,
) -> Result<KillProcessResult, AppError> {
    audit_log(&audit_logger_state, "kill_process", &serde_json::to_value(params)?).await;

    let mut sys_guard = sysinfo_state.lock().await;
    let pid_to_kill = Pid::from(params.pid);
    debug!(target_pid = ?pid_to_kill, "Attempting to kill process");

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

    // Try SIGTERM (graceful termination)
    if let Some(p_term) = sys_guard.process(pid_to_kill) {
        if p_term.kill_with(Signal::Term).unwrap_or(false) {
            // Short delay to allow process to terminate
            tokio::time::sleep(Duration::from_millis(200)).await;
            sys_guard.refresh_process_specifics(pid_to_kill, ProcessRefreshKind::everything());
            if sys_guard.process(pid_to_kill).is_none() {
                debug!(pid = ?pid_to_kill, "Process terminated with SIGTERM");
                return Ok(KillProcessResult {
                    success: true,
                    message: format!("Process {} ({}) terminated successfully with SIGTERM.", params.pid, process_name_for_message),
                });
            }
            debug!(pid = ?pid_to_kill, "Process still alive after SIGTERM attempt.");
        } else {
            debug!(pid = ?pid_to_kill, "Sending SIGTERM failed or process already gone.");
        }
    }

    // Try SIGKILL (forceful termination)
    sys_guard.refresh_process_specifics(pid_to_kill, ProcessRefreshKind::everything());
    if let Some(p_kill) = sys_guard.process(pid_to_kill) {
        if p_kill.kill_with(Signal::Kill).unwrap_or(false) {
            tokio::time::sleep(Duration::from_millis(100)).await;
            sys_guard.refresh_process_specifics(pid_to_kill, ProcessRefreshKind::everything());
            if sys_guard.process(pid_to_kill).is_none() {
                debug!(pid = ?pid_to_kill, "Process terminated with SIGKILL");
                return Ok(KillProcessResult {
                    success: true,
                    message: format!("Process {} ({}) terminated successfully with SIGKILL.", params.pid, process_name_for_message),
                });
            } else {
                warn!(pid = ?pid_to_kill, "Process still running after SIGKILL");
                return Ok(KillProcessResult {
                    success: false,
                    message: format!("Sent SIGKILL to process {} ({}), but it might still be running. OS may prevent killing certain processes.", params.pid, process_name_for_message),
                });
            }
        } else {
            warn!(pid = ?pid_to_kill, "Failed to send SIGKILL");
            sys_guard.refresh_process_specifics(pid_to_kill, ProcessRefreshKind::everything());
            if sys_guard.process(pid_to_kill).is_none() {
                 return Ok(KillProcessResult {
                    success: true, // Considered success if it's gone
                    message: format!("Process {} ({}) no longer found after failed SIGKILL attempt, likely terminated by previous SIGTERM or exited.", params.pid, process_name_for_message),
                });
            }
            return Ok(KillProcessResult {
                success: false,
                message: format!("Failed to send SIGKILL to process {} ({}). It might require higher privileges or be a system process.", params.pid, process_name_for_message),
            });
        }
    } else {
        debug!(pid = ?pid_to_kill, "Process not found before SIGKILL attempt, assuming terminated by SIGTERM or exited.");
        return Ok(KillProcessResult {
            success: true,
            message: format!("Process {} ({}) no longer found, likely terminated.", params.pid, process_name_for_message),
        });
    }
}