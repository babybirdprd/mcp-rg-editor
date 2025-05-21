use crate::config::Config;
use crate::error::AppError;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tauri::State;
use tracing::{info, warn};
use regex::Regex; // For set_config_value if needed

#[derive(serde::Deserialize)]
pub struct SetConfigValuePayload {
    key: String,
    value: Value,
}

#[tauri::command]
pub async fn get_config_command(
    config_state: State<'_, Arc<RwLock<Config>>>,
) -> Result<Config, AppError> {
    let config_guard = config_state.read().map_err(|e| {
        AppError::ConfigError(format!("Failed to acquire read lock on config: {}", e))
    })?;
    // Clone the config to return it. Config struct needs to derive Serialize.
    Ok(config_guard.clone())
}

#[tauri::command]
pub async fn set_config_value_command(
    payload: SetConfigValuePayload,
    config_state: State<'_, Arc<RwLock<Config>>>,
    app_handle: tauri::AppHandle, // For re-configuring plugin scopes if needed
) -> Result<String, AppError> {
    let mut config_guard = config_state.write().map_err(|e| {
        AppError::ConfigError(format!("Failed to acquire write lock on config: {}", e))
    })?;

    let key = payload.key.as_str();
    let value_to_set = payload.value;
    let mut update_applied = true;
    let mut requires_scope_reconfig = false;

    info!(key = %key, value = ?value_to_set, "Attempting to set config value");

    match key {
        "allowedDirectories" => {
            let new_dirs: Result<Vec<PathBuf>, _> = match value_to_set {
                Value::Array(arr_val) => arr_val.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .map(|s| crate::config::expand_tilde(&s).map_err(|e| AppError::InvalidPath(format!("Invalid path in allowedDirectories {}: {}", s, e))))
                    .collect(),
                Value::String(str_val) => str_val.split(',')
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .map(|s| crate::config::expand_tilde(s).map_err(|e| AppError::InvalidPath(format!("Invalid path in allowedDirectories {}: {}", s, e))))
                    .collect(),
                _ => {
                    update_applied = false;
                    Err(AppError::InvalidInputArgument("allowedDirectories must be a string or an array of strings".to_string()))
                }
            };
            match new_dirs {
                Ok(mut dirs) => {
                    // Ensure FILES_ROOT is always allowed if not a broad root
                    let is_files_root_broad = config_guard.files_root == PathBuf::from("/") ||
                                            (cfg!(windows) && config_guard.files_root.parent().is_none() && config_guard.files_root.is_absolute());
                    if !is_files_root_broad && !dirs.iter().any(|ad| ad == &config_guard.files_root) {
                        dirs.push(config_guard.files_root.clone());
                    }
                    dirs.sort();
                    dirs.dedup();
                    config_guard.allowed_directories = dirs;
                    requires_scope_reconfig = true;
                },
                Err(e) => return Err(e),
            }
        },
        "blockedCommands" => {
            let new_cmds: Result<Vec<String>, _> = match value_to_set {
                Value::Array(arr_val) => Ok(arr_val.iter().filter_map(|v| v.as_str().map(String::from)).collect()),
                Value::String(str_val) => Ok(str_val.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()),
                _ => {
                    update_applied = false;
                    Err(AppError::InvalidInputArgument("blockedCommands must be a string or an array of strings".to_string()))
                }
            };
            match new_cmds {
                Ok(cmds) => config_guard.blocked_commands = cmds,
                Err(e) => return Err(e),
            }
        },
        "defaultShell" => {
            if let Some(str_val) = value_to_set.as_str() {
                config_guard.default_shell = Some(str_val.to_string());
            } else if value_to_set.is_null() {
                 config_guard.default_shell = None;
            } else {
                update_applied = false;
                warn!(key=key, "set_config_value: value for string key was not a string or null");
            }
        },
        "fileReadLineLimit" => {
            if let Some(num_val) = value_to_set.as_u64() {
                config_guard.file_read_line_limit = num_val as usize;
            } else {
                update_applied = false;
                warn!(key=key, "set_config_value: value for u64 key was not u64");
            }
        },
         "fileWriteLineLimit" => {
            if let Some(num_val) = value_to_set.as_u64() {
                config_guard.file_write_line_limit = num_val as usize;
            } else {
                update_applied = false;
                warn!(key=key, "set_config_value: value for u64 key was not u64");
            }
        },
        // FILES_ROOT and log paths are generally not recommended to be changed at runtime
        // due to complexity with existing file handles, watcher scopes etc.
        "filesRoot" | "mcpLogDir" | "auditLogFile" | "fuzzySearchLogFile" => {
             warn!(key=key, "set_config_value: Dynamically changing this path is not recommended or supported.");
             return Err(AppError::ConfigError(format!("Configuration key '{}' cannot be changed at runtime through this command.", key)));
        }
        _ => {
            update_applied = false;
            warn!(key=key, "set_config_value: Unknown or unhandled config key");
            return Err(AppError::InvalidInputArgument(format!("Unknown or read-only config key: {}", key)));
        }
    }

    if requires_scope_reconfig {
        // This is a simplified example. Real scope update is more complex.
        // You might need to restart parts of the app or re-initialize plugins.
        // For fs scope, you'd typically define it broadly in tauri.conf.json and rely on
        // Rust-side validation against config_guard.allowed_directories.
        // Dynamically changing plugin scopes at runtime is not straightforward.
        warn!("Configuration affecting FS scope changed. Manual path validation will enforce new 'allowedDirectories'. Full dynamic scope update for tauri-plugin-fs is complex and may require app restart or plugin re-initialization for strict enforcement at plugin level.");
        // Example: Re-apply FS scope (conceptual - actual API might differ or not exist for dynamic updates)
        // let fs_scope = app_handle.fs_scope();
        // fs_scope.set_allowed_paths(&config_guard.allowed_directories); // This is hypothetical
    }


    if update_applied {
        info!(key = %key, "Successfully set config value");
        Ok(format!("Successfully set config key '{}'. Changes are in-memory for the current session.", key))
    } else {
        Err(AppError::InvalidInputArgument(format!("Invalid value type for config key '{}'", key)))
    }
}