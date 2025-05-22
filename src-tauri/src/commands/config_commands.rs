use crate::config::{Config, expand_tilde};
use crate::error::AppError;
use crate::utils::audit_logger::audit_log;

use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Arc, RwLock as StdRwLock};
use tauri::{AppHandle, State};
use tracing::{info, warn};


#[derive(serde::Deserialize, serde::Serialize)]
pub struct SetConfigValuePayload {
    key: String,
    value: Value,
}

#[tauri::command(async)]
pub async fn get_config_command(
    config_state: State<'_, Arc<StdRwLock<Config>>>,
    audit_logger_state: State<'_, Arc<crate::utils::audit_logger::AuditLogger>>,
) -> Result<Config, String> {
    audit_log(&audit_logger_state, "ui_get_config", &serde_json::Value::Null).await;

    let config_guard = config_state.read().map_err(|e| {
        AppError::ConfigError(format!("Failed to acquire read lock on config: {}", e)).to_string()
    })?;
    Ok(config_guard.clone())
}

#[tauri::command(async)]
pub async fn set_config_value_command(
    _app_handle: AppHandle,
    payload: SetConfigValuePayload,
    config_state: State<'_, Arc<StdRwLock<Config>>>,
    audit_logger_state: State<'_, Arc<crate::utils::audit_logger::AuditLogger>>,
) -> Result<String, String> {
    audit_log(&audit_logger_state, "ui_set_config_value", &serde_json::to_value(&payload).unwrap_or_default()).await;

    let mut config_guard = config_state.write().map_err(|e| {
        AppError::ConfigError(format!("Failed to acquire write lock on config: {}", e)).to_string()
    })?;

    let key = payload.key.as_str();
    let value_to_set = payload.value;
    // let mut _update_applied = true; // No longer strictly needed due to early returns

    info!(key = %key, value = ?value_to_set, "UI: Attempting to set config value");

    match key {
        "allowedDirectories" => {
            let new_dirs_str_values: Vec<String> = match value_to_set {
                Value::Array(arr_val) => arr_val.into_iter().filter_map(|v| v.as_str().map(String::from)).collect(),
                Value::String(str_val) => str_val.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
                _ => return Err(AppError::InvalidInputArgument("allowedDirectories must be a JSON array of strings or a comma-separated string".to_string()).to_string()),
            };

            let mut new_dirs_pathbuf = Vec::new();
            for s_path in new_dirs_str_values {
                if s_path.is_empty() { continue; }
                match expand_tilde(&s_path) {
                    Ok(p) => new_dirs_pathbuf.push(p.canonicalize().unwrap_or(p)),
                    Err(e) => return Err(AppError::InvalidPath(format!("Invalid path in allowedDirectories '{}': {}", s_path, e)).to_string()),
                }
            }

            let is_files_root_broad = config_guard.files_root == PathBuf::from("/") ||
                                    (cfg!(windows) && config_guard.files_root.parent().is_none() && config_guard.files_root.is_absolute());
            if !is_files_root_broad && !new_dirs_pathbuf.iter().any(|ad| ad == &config_guard.files_root) {
                new_dirs_pathbuf.push(config_guard.files_root.clone());
            }
            new_dirs_pathbuf.sort();
            new_dirs_pathbuf.dedup();
            config_guard.allowed_directories = new_dirs_pathbuf;
            info!(new_allowed_dirs = ?config_guard.allowed_directories, "Updated allowedDirectories");
        },
        "blockedCommands" => {
            let new_cmds: Vec<String> = match value_to_set {
                Value::Array(arr_val) => arr_val.into_iter().filter_map(|v| v.as_str().map(String::from)).collect(),
                Value::String(str_val) => str_val.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
                _ => return Err(AppError::InvalidInputArgument("blockedCommands must be a JSON array of strings or a comma-separated string".to_string()).to_string()),
            };
            config_guard.blocked_commands = new_cmds;
            info!(new_blocked_cmds = ?config_guard.blocked_commands, "Updated blockedCommands");
        },
        "defaultShell" => {
            if let Some(str_val) = value_to_set.as_str() {
                config_guard.default_shell = if str_val.trim().is_empty() { None } else { Some(str_val.trim().to_string()) };
            } else if value_to_set.is_null() {
                 config_guard.default_shell = None;
            } else {
                warn!(key=key, "set_config_value: value for defaultShell was not a string or null");
                 return Err(AppError::InvalidInputArgument(format!("Invalid value type for config key '{}'", key)).to_string());
            }
            info!(new_default_shell = ?config_guard.default_shell, "Updated defaultShell");
        },
        "logLevel" => {
            if let Some(str_val) = value_to_set.as_str() {
                config_guard.log_level = str_val.to_string();
                warn!("Log level set to '{}' in config. A full application restart may be needed for tracing subscriber changes to take complete effect.", str_val);
            } else {
                warn!(key=key, "set_config_value: value for logLevel was not a string");
                return Err(AppError::InvalidInputArgument(format!("Invalid value type for config key '{}'", key)).to_string());
            }
            info!(new_log_level = %config_guard.log_level, "Updated logLevel");
        },
        "fileReadLineLimit" => {
            if let Some(num_val) = value_to_set.as_u64() {
                config_guard.file_read_line_limit = num_val as usize;
            } else {
                warn!(key=key, "set_config_value: value for fileReadLineLimit was not u64");
                return Err(AppError::InvalidInputArgument(format!("Invalid value type for config key '{}'", key)).to_string());
            }
            info!(new_read_limit = %config_guard.file_read_line_limit, "Updated fileReadLineLimit");
        },
         "fileWriteLineLimit" => {
            if let Some(num_val) = value_to_set.as_u64() {
                config_guard.file_write_line_limit = num_val as usize;
            } else {
                warn!(key=key, "set_config_value: value for fileWriteLineLimit was not u64");
                 return Err(AppError::InvalidInputArgument(format!("Invalid value type for config key '{}'", key)).to_string());
            }
            info!(new_write_limit = %config_guard.file_write_line_limit, "Updated fileWriteLineLimit");
        },
        "filesRoot" | "mcpLogDir" | "auditLogFile" | "fuzzySearchLogFile" => {
             warn!(key=key, "set_config_value: Dynamically changing this path is not supported via this command.");
             return Err(AppError::ConfigError(format!("Configuration key '{}' cannot be changed at runtime through this command.", key)).to_string());
        }
        _ => {
            warn!(key=key, "set_config_value: Unknown or unhandled config key");
            return Err(AppError::InvalidInputArgument(format!("Unknown or read-only config key: {}", key)).to_string());
        }
    }

    info!(key = %key, "Successfully set config value via UI command");
    Ok(format!("Successfully set config key '{}'. Changes are in-memory for the current session.", key))
}