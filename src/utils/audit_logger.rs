use crate::config::Config;
use anyhow::Result;
use chrono::Utc;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;
use tracing::error;

pub struct AuditLogger {
    log_file_path: PathBuf,
    max_size_bytes: u64,
}

impl AuditLogger {
    pub fn new(config: Arc<Config>) -> Self {
        // Ensure log directory exists
        if let Some(parent_dir) = config.audit_log_file.parent() {
            if !parent_dir.exists() {
                if let Err(e) = std::fs::create_dir_all(parent_dir) {
                    error!(path = %parent_dir.display(), error = %e, "Failed to create audit log directory");
                }
            }
        }
        Self {
            log_file_path: config.audit_log_file.clone(),
            max_size_bytes: config.audit_log_max_size_bytes,
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

    pub async fn log_tool_call(&self, tool_name: &str, arguments: &Value) {
        if let Err(e) = self.try_log_tool_call(tool_name, arguments).await {
            error!(tool = %tool_name, error = %e, "Failed to write audit log");
        }
    }

    async fn try_log_tool_call(&self, tool_name: &str, arguments: &Value) -> Result<()> {
        self.rotate_log_if_needed().await?;

        let timestamp = Utc::now().to_rfc3339();
        
        // Sanitize arguments for logging - primarily to avoid logging large file contents
        let mut sanitized_args = arguments.clone();
        if let Some(obj) = sanitized_args.as_object_mut() {
            if let Some(content_val) = obj.get_mut("content") {
                if content_val.is_string() && content_val.as_str().unwrap_or("").len() > 1024 { // Arbitrary limit for "large"
                    *content_val = Value::String("<content truncated for log>".to_string());
                }
            }
             if let Some(old_string_val) = obj.get_mut("old_string") {
                if old_string_val.is_string() && old_string_val.as_str().unwrap_or("").len() > 1024 {
                    *old_string_val = Value::String("<old_string truncated for log>".to_string());
                }
            }
            if let Some(new_string_val) = obj.get_mut("new_string") {
                if new_string_val.is_string() && new_string_val.as_str().unwrap_or("").len() > 1024 {
                    *new_string_val = Value::String("<new_string truncated for log>".to_string());
                }
            }
        }


        let args_string = serde_json::to_string(&sanitized_args)?;
        let log_entry = format!("{} | {:<20} | Arguments: {}\n", timestamp, tool_name, args_string);

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.log_file_path)
            .await?;

        file.write_all(log_entry.as_bytes()).await?;
        Ok(())
    }
}