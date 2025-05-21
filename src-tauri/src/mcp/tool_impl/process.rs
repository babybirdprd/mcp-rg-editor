// FILE: src-tauri/src/mcp/tool_impl/process.rs
// IMPORTANT NOTE: Rewrite the entire file.
use crate::error::AppError;
use crate::mcp::handler::ToolDependencies; // Correctly use ToolDependencies
use serde::{Deserialize, Serialize};
use sysinfo::{Pid, Signal, ProcessRefreshKind, Uid, System as SysinfoSystem}; // Added System
use tokio::sync::MutexGuard; // For working with the MutexGuard
use tracing::{debug, instrument, warn};

// --- MCP Specific Parameter Structs ---
#[derive(Debug, Deserialize)]
pub struct KillProcessParamsMCP { pub pid: usize; }

// --- MCP Specific Result Structs ---
#[derive(Debug, Serialize)]
pub struct ProcessInfoMCP {
    pid: String, name: String, cpu_usage: f32, memory_mb: u64,
    command: String, status: String, user: Option<String>, start_time_epoch_secs: u64,
}
#[derive(Debug, Serialize)]
pub struct KillProcessResultMCP { pub success: bool, pub message: String; }

fn format_uid_mcp(uid_opt: Option<&Uid>) -> Option<String> {
    uid_opt.map(|uid| uid.to_string())
}

#[instrument(skip(deps))]
pub async fn mcp_list_processes(deps: &ToolDependencies) -> Result<Vec<ProcessInfoMCP>, AppError> {
    let mut sys_guard: MutexGuard<'_, SysinfoSystem> = deps.sysinfo_state.lock().await;
    sys_guard.refresh_processes_specifics(ProcessRefreshKind::everything());
    debug!("MCP Tool: Listing {} system processes.", sys_guard.processes().len());
    Ok(sys_guard.processes().iter().map(|(p, process)| ProcessInfoMCP {
        pid: p.as_u32().to_string(), name: process.name().to_string(), cpu_usage: process.cpu_usage(),
        memory_mb: process.memory() / (1024 * 1024), command: process.cmd().join(" "),
        status: process.status().to_string(), user: format_uid_mcp(process.user_id()),
        start_time_epoch_secs: process.start_time(),
    }).collect())
}

#[instrument(skip(deps, params), fields(pid = %params.pid))]
pub async fn mcp_kill_process(deps: &ToolDependencies, params: KillProcessParamsMCP) -> Result<KillProcessResultMCP, AppError> {
    let mut sys_guard: MutexGuard<'_, SysinfoSystem> = deps.sysinfo_state.lock().await;
    let pid_to_kill = Pid::from(params.pid);
    sys_guard.refresh_process_specifics(pid_to_kill, ProcessRefreshKind::everything());
    let proc_name = match sys_guard.process(pid_to_kill) {
        Some(p) => p.name().to_string(),
        None => return Ok(KillProcessResultMCP { success: false, message: format!("PID {} not found.", params.pid) }),
    };

    if let Some(p) = sys_guard.process(pid_to_kill) {
        if p.kill_with(Signal::Term).unwrap_or(false) {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            sys_guard.refresh_process_specifics(pid_to_kill, ProcessRefreshKind::everything());
            if sys_guard.process(pid_to_kill).is_none() { return Ok(KillProcessResultMCP { success: true, message: format!("PID {} ({}) terminated with SIGTERM.", params.pid, proc_name) }); }
            debug!(pid = ?pid_to_kill, "Process still alive after SIGTERM.");
        } else {
            debug!(pid = ?pid_to_kill, "Sending SIGTERM failed or process already gone.");
        }
    }

    sys_guard.refresh_process_specifics(pid_to_kill, ProcessRefreshKind::everything()); // Refresh again
    if let Some(p) = sys_guard.process(pid_to_kill) {
        if p.kill_with(Signal::Kill).unwrap_or(false) {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            sys_guard.refresh_process_specifics(pid_to_kill, ProcessRefreshKind::everything());
            if sys_guard.process(pid_to_kill).is_none() { return Ok(KillProcessResultMCP { success: true, message: format!("PID {} ({}) terminated with SIGKILL.", params.pid, proc_name) }); }
            else {
                warn!(pid = ?pid_to_kill, "Process still running after SIGKILL.");
                return Ok(KillProcessResultMCP { success: false, message: format!("Sent SIGKILL to PID {} ({}), but it may still be running.", params.pid, proc_name) });
            }
        } else {
            warn!(pid = ?pid_to_kill, "Failed to send SIGKILL.");
            sys_guard.refresh_process_specifics(pid_to_kill, ProcessRefreshKind::everything()); // Final check
            if sys_guard.process(pid_to_kill).is_none() {
                return Ok(KillProcessResultMCP { success: true, message: format!("PID {} ({}) no longer found after failed SIGKILL, likely terminated.", params.pid, proc_name) });
            }
            return Ok(KillProcessResultMCP { success: false, message: format!("Failed to send SIGKILL to PID {} ({}).", params.pid, proc_name) });
        }
    } else { // Process not found before SIGKILL attempt
        debug!(pid = ?pid_to_kill, "Process not found before SIGKILL, assuming terminated.");
        return Ok(KillProcessResultMCP { success: true, message: format!("PID {} ({}) no longer found, likely terminated.", params.pid, proc_name) });
    }
}