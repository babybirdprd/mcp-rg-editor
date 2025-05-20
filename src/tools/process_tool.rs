use crate::config::Config;
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use sysinfo::{Pid, ProcessExt, System, SystemExt};
use tracing::instrument;


#[derive(Debug, Deserialize)]
pub struct KillProcessParams {
    pub pid: usize, // sysinfo uses usize for PIDs
}

#[derive(Debug, Serialize)]
pub struct ProcessInfo {
    pid: String, // Store as string for consistency as PIDs can be large
    name: String,
    cpu_usage: f32,
    memory_mb: u64,
    command: String,
}

#[derive(Debug, Serialize)]
pub struct KillProcessResult {
    pub success: bool,
    pub message: String,
}

#[derive(Debug)]
pub struct ProcessManager {
    // config might be needed if we add features like filtering by allowed commands, etc.
    _config: Arc<Config>,
    system: Arc<Mutex<System>>, // Mutex for interior mutability of System
}
use std::sync::Mutex; // Use std::sync::Mutex as sysinfo is sync

impl ProcessManager {
    pub fn new(config: Arc<Config>) -> Self {
        let mut sys = System::new_all();
        sys.refresh_all(); // Initial refresh
        Self {
            _config: config,
            system: Arc::new(Mutex::new(sys)),
        }
    }

    #[instrument(skip(self))]
    pub async fn list_processes(&self) -> Result<Vec<ProcessInfo>, AppError> {
        let mut sys = self.system.lock().map_err(|_| AppError::ProcessError("Failed to lock system mutex".to_string()))?;
        sys.refresh_processes(); // Refresh process list

        let mut processes = Vec::new();
        for (pid, process) in sys.processes() {
            processes.push(ProcessInfo {
                pid: pid.to_string(),
                name: process.name().to_string(),
                cpu_usage: process.cpu_usage(),
                memory_mb: process.memory() / (1024 * 1024), // Convert bytes to MB
                command: process.cmd().join(" "),
            });
        }
        Ok(processes)
    }

    #[instrument(skip(self, params), fields(pid = %params.pid))]
    pub async fn kill_process(&self, params: &KillProcessParams) -> Result<KillProcessResult, AppError> {
        let mut sys = self.system.lock().map_err(|_| AppError::ProcessError("Failed to lock system mutex".to_string()))?;
        sys.refresh_processes(); // Ensure process list is up-to-date
        
        let pid_to_kill = Pid::from(params.pid);

        if let Some(process) = sys.process(pid_to_kill) {
            if process.kill() {
                Ok(KillProcessResult {
                    success: true,
                    message: format!("Process {} ({}) terminated.", params.pid, process.name()),
                })
            } else {
                Ok(KillProcessResult {
                    success: false,
                    message: format!("Failed to terminate process {} ({}). Signal sent, but termination could not be confirmed or process does not exist.", params.pid, process.name()),
                })
            }
        } else {
            Ok(KillProcessResult {
                success: false,
                message: format!("Process with PID {} not found.", params.pid),
            })
        }
    }
}