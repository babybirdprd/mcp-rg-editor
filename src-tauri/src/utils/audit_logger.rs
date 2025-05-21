// FILE: src-tauri/src/utils/audit_logger.rs
// IMPORTANT NOTE: Rewrite the entire file.
use crate::config::Config;
use anyhow::Result;
use chrono::Utc;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Arc, RwLock as StdRwLock};
use tauri::State;
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;
use tracing::error;

#[derive(Debug)]
pub struct AuditLogger {
    log_file_path: PathBuf,
    max_size_bytes: u64,
}

impl AuditLogger {
    pub fn new(config_state: Arc<StdRwLock<Config>>) -> Self {
        let config_guard = config_state.read().unwrap();
        let log_file_path = config_guard.audit_log_file.clone();
        let max_size_bytes = config_guard.audit_log_max_size_bytes;
        drop(config_guard);

        if let Some(parent_dir) = log_file_path.parent() {
            if !parent_dir.exists() {
                if let Err(e) = std::fs::create_dir_all(parent_dir) {
                    error!(path = %parent_dir.display(), error = %e, "Failed to create audit log directory");
                }
            }
        }
        Self {
            log_file_path,
            max_size_bytes,
        }
    }

    async fn rotate_log_if_needed(&self) -> Result<()> {
        if !self.log_file_path.exists() {
            return Ok(());
        }

        let metadata = fs::metadata(&self.log_file_path).await?;
        if metadata.len() >= self.max_size_bytes {
            let timestamp = Utc::now().format("%Y-%m-%d_%H-%M-%S").to_string();
            let file_stem = self
                .log_file_path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy();
            let extension = self
                .log_file_path
                .extension()
                .unwrap_or_default()
                .to_string_lossy();

            let backup_file_name = format!("{}_{}.{}", file_stem, timestamp, extension);
            let backup_path = self.log_file_path.with_file_name(backup_file_name);

            fs::rename(&self.log_file_path, backup_path).await?;
        }
        Ok(())
    }

    pub async fn log_command_call(&self, command_name: &str, arguments: &Value) {
        if let Err(e) = self.try_log_command_call(command_name, arguments).await {
            error!(command = %command_name, error = %e, "Failed to write audit log");
        }
    }

    async fn try_log_command_call(&self, command_name: &str, arguments: &Value) -> Result<()> {
        self.rotate_log_if_needed().await?;

        let timestamp = Utc::now().to_rfc3339();

        let mut sanitized_args = arguments.clone();
        if let Some(obj) = sanitized_args.as_object_mut() {
            for key_to_sanitize in ["content", "old_string", "new_string", "command", "pattern"] { // Added "pattern"
                if let Some(val_mut) = obj.get_mut(key_to_sanitize) {
                    if val_mut.is_string() && val_mut.as_str().unwrap_or("").len() > 256 {
                        *val_mut = Value::String(format!("<{} truncated for log>", key_to_sanitize));
                    }
                }
            }
        }

        let args_string = serde_json::to_string(&sanitized_args)?;
        let log_entry = format!("{} | CMD: {:<25} | Arguments: {}\n", timestamp, command_name, args_string);

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_file_path)
            .await?;

        file.write_all(log_entry.as_bytes()).await?;
        Ok(())
    }
}

pub async fn audit_log(
    logger_state: &State<'_, Arc<AuditLogger>>, // Borrow State directly
    command_name: &str,
    arguments: &Value,
) {
    logger_state.inner().log_command_call(command_name, arguments).await;
}