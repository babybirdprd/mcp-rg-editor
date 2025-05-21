// FILE: src-tauri/src/commands/config_commands.rs
// IMPORTANT NOTE: Rewrite the entire file.
use crate::config::{Config, expand_tilde}; // expand_tilde is used here
use crate::error::AppError;
use crate::utils::audit_logger::audit_log; // For logging command calls

use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tauri::{AppHandle, Manager, State}; // Manager for app_handle.fs_scope() if needed
use tracing::{info, warn};
// regex::Regex is not directly used here as Config stores strings,
// and compilation to Regex happens on demand within Config or tools.

#[derive(serde::Deserialize, serde::Serialize)] // Added Serialize for audit log
pub struct SetConfigValuePayload {
    key: String,
    value: Value, // Frontend sends JSON Value
}

#[tauri::command(async)]
pub async fn get_config_command(
    config_state: State<'_, Arc<RwLock<Config>>>,
    audit_logger_state: State<'_, Arc<crate::utils::audit_logger::AuditLogger>>,
) -> Result<Config, String> { // Return String for error to match Tauri's Invoke error handling
    // Audit this internal UI command call as well
    match audit_log(&audit_logger_state, "ui_get_config", &serde_json::Value::Null).await {
        Ok(_) => {},
        Err(e) => warn!("Failed to audit ui_get_config: {}", e), // Log audit failure but proceed
    };

    let config_guard = config_state.read().map_err(|e| {
        AppError::ConfigError(format!("Failed to acquire read lock on config: {}", e)).to_string()
    })?;
    Ok(config_guard.clone()) // Config needs to derive Serialize
}

#[tauri::command(async)]
pub async fn set_config_value_command(
    app_handle: AppHandle, // Keep for potential future use (e.g., dynamic scope updates)
    payload: SetConfigValuePayload,
    config_state: State<'_, Arc<RwLock<Config>>>,
    audit_logger_state: State<'_, Arc<crate::utils::audit_logger::AuditLogger>>,
) -> Result<String, String> { // Return String for success message or error
    match audit_log(&audit_logger_state, "ui_set_config_value", &serde_json::to_value(&payload).unwrap_or_default()).await {
        Ok(_) => {},
        Err(e) => warn!("Failed to audit ui_set_config_value: {}", e),
    };

    let mut config_guard = config_state.write().map_err(|e| {
        AppError::ConfigError(format!("Failed to acquire write lock on config: {}", e)).to_string()
    })?;

    let key = payload.key.as_str();
    let value_to_set = payload.value;
    let mut update_applied = true;
    // let mut requires_scope_reconfig = false; // For future if dynamic FS scope changes are implemented

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
                if s_path.is_empty() { continue; } // Skip empty strings that might result from " ,, "
                match expand_tilde(&s_path) {
                    Ok(p) => new_dirs_pathbuf.push(p.canonicalize().unwrap_or(p)), // Store canonicalized or as-is if non-existent
                    Err(e) => return Err(AppError::InvalidPath(format!("Invalid path in allowedDirectories '{}': {}", s_path, e)).to_string()),
                }
            }

            // Ensure FILES_ROOT is always allowed if not a broad root
            let is_files_root_broad = config_guard.files_root == PathBuf::from("/") ||
                                    (cfg!(windows) && config_guard.files_root.parent().is_none() && config_guard.files_root.is_absolute());
            if !is_files_root_broad && !new_dirs_pathbuf.iter().any(|ad| ad == &config_guard.files_root) {
                new_dirs_pathbuf.push(config_guard.files_root.clone());
            }
            new_dirs_pathbuf.sort();
            new_dirs_pathbuf.dedup();
            config_guard.allowed_directories = new_dirs_pathbuf;
            // requires_scope_reconfig = true;
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
                update_applied = false;
                warn!(key=key, "set_config_value: value for defaultShell was not a string or null");
            }
            info!(new_default_shell = ?config_guard.default_shell, "Updated defaultShell");
        },
        "logLevel" => {
            if let Some(str_val) = value_to_set.as_str() {
                config_guard.log_level = str_val.to_string();
                warn!("Log level set to '{}' in config. A full application restart may be needed for tracing subscriber changes to take complete effect.", str_val);
                // Potentially try to update tauri-plugin-log dynamically if its API supports it
                // For now, this just updates the config value.
            } else {
                update_applied = false;
                warn!(key=key, "set_config_value: value for logLevel was not a string");
            }
            info!(new_log_level = %config_guard.log_level, "Updated logLevel");
        },
        "fileReadLineLimit" => {
            if let Some(num_val) = value_to_set.as_u64() {
                config_guard.file_read_line_limit = num_val as usize;
            } else {
                update_applied = false;
                warn!(key=key, "set_config_value: value for fileReadLineLimit was not u64");
            }
            info!(new_read_limit = %config_guard.file_read_line_limit, "Updated fileReadLineLimit");
        },
         "fileWriteLineLimit" => {
            if let Some(num_val) = value_to_set.as_u64() {
                config_guard.file_write_line_limit = num_val as usize;
            } else {
                update_applied = false;
                warn!(key=key, "set_config_value: value for fileWriteLineLimit was not u64");
            }
            info!(new_write_limit = %config_guard.file_write_line_limit, "Updated fileWriteLineLimit");
        },
        "filesRoot" | "mcpLogDir" | "auditLogFile" | "fuzzySearchLogFile" => {
             warn!(key=key, "set_config_value: Dynamically changing this path is not supported via this command.");
             return Err(AppError::ConfigError(format!("Configuration key '{}' cannot be changed at runtime through this command.", key)).to_string());
        }
        _ => {
            update_applied = false;
            warn!(key=key, "set_config_value: Unknown or unhandled config key");
            return Err(AppError::InvalidInputArgument(format!("Unknown or read-only config key: {}", key)).to_string());
        }
    }

    // if requires_scope_reconfig {
    //     warn!("Configuration affecting FS scope changed. Rust-side validation will use new 'allowedDirectories'. Dynamic tauri-plugin-fs scope update is complex and may require app restart or plugin re-initialization for strict enforcement at plugin level.");
    //     // Example: app_handle.fs_scope().set_allowed_paths(&config_guard.allowed_directories); // This is hypothetical
    // }

    if update_applied {
        info!(key = %key, "Successfully set config value via UI command");
        Ok(format!("Successfully set config key '{}'. Changes are in-memory for the current session.", key))
    } else {
        Err(AppError::InvalidInputArgument(format!("Invalid value type for config key '{}'", key)).to_string())
    }
}