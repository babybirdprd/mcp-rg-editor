// FILE: src-tauri/src/commands/filesystem_commands.rs
// IMPORTANT NOTE: Rewrite the entire file.
use crate::config::Config;
use crate::error::AppError;
use crate::utils::path_utils::validate_and_normalize_path;
use crate::utils::line_ending_handler::{detect_line_ending, normalize_line_endings, LineEndingStyle};
use crate::utils::audit_logger::audit_log;

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, RwLock as StdRwLock, RwLockReadGuard};
use tauri::{AppHandle, Manager, State};
use tokio::time::{timeout, Duration};
use tracing::{debug, instrument, warn, error};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};

// --- Request Structs ---
#[derive(Debug, Deserialize, Serialize)]
pub struct ReadFileParams {
    pub path: String,
    #[serde(default)]
    pub is_url: bool,
    #[serde(default)]
    pub offset: usize,
    pub length: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ReadMultipleFilesParams {
    pub paths: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WriteFileParams {
    pub path: String,
    pub content: String,
    #[serde(default = "default_rewrite_mode")]
    pub mode: WriteMode,
}
fn default_rewrite_mode() -> WriteMode { WriteMode::Rewrite }

#[derive(Debug, Deserialize, PartialEq, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WriteMode {
    Rewrite,
    Append,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CreateDirectoryParams {
    pub path: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ListDirectoryParams {
    pub path: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MoveFileParams {
    pub source: String,
    pub destination: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct GetFileInfoParams {
    pub path: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct SearchFilesParams {
    pub path: String,
    pub pattern: String,
    #[serde(rename = "timeoutMs")]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub recursive: bool,
    #[serde(default = "default_search_max_depth")]
    pub max_depth: usize,
}
fn default_search_max_depth() -> usize { 10 }


// --- Response Structs ---
#[derive(Debug, Serialize)]
pub struct FileContent {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_data_base64: Option<String>,
    pub mime_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines_read: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_lines: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ReadMultipleFilesResult {
    pub results: Vec<FileContent>,
}

#[derive(Debug, Serialize)]
pub struct FileOperationResult {
    pub success: bool,
    pub path: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct ListDirectoryResult {
    pub path: String,
    pub entries: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct FileInfoResult {
    pub path: String,
    pub size: u64,
    pub is_dir: bool,
    pub is_file: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_iso: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_iso: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accessed_iso: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permissions_octal: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SearchFilesResult {
    pub path: String,
    pub pattern: String,
    pub matches: Vec<String>,
    pub timed_out: bool,
}

const URL_FETCH_TIMEOUT_MS: u64 = 30000;
const FILE_SEARCH_TIMEOUT_MS: u64 = 30000;

fn is_image_mime(mime_type: &str) -> bool {
    mime_type.starts_with("image/") &&
    (mime_type.ends_with("/png") || mime_type.ends_with("/jpeg") || mime_type.ends_with("/gif") || mime_type.ends_with("/webp"))
}

async fn read_file_from_url_internal(
    http_client: &reqwest::Client, // Use provided client
    url_str: &str,
    _app_handle: &AppHandle, // Keep for consistency or future http plugin use
) -> Result<FileContent, AppError> {
    debug!(url = %url_str, "Reading file from URL via reqwest");

    let response = match tokio::time::timeout(
        std::time::Duration::from_millis(URL_FETCH_TIMEOUT_MS),
        http_client.get(url_str).send()
    ).await {
        Ok(Ok(resp)) => resp,
        Ok(Err(e)) => return Err(AppError::ReqwestError(e.to_string())),
        Err(_) => return Err(AppError::TimeoutError(format!("URL fetch timed out after {}ms: {}", URL_FETCH_TIMEOUT_MS, url_str))),
    };

    if !response.status().is_success() {
        return Err(AppError::ReqwestError(
            response.error_for_status().map_err(|e| e.to_string()).unwrap_or_else(|e| format!("HTTP error for {}: {}", url_str, e))
        ));
    }

    let mime_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("application/octet-stream")
        .split(';')
        .next()
        .unwrap_or("application/octet-stream")
        .trim()
        .to_string();

    if is_image_mime(&mime_type) {
        let bytes = response.bytes().await.map_err(|e| AppError::ReqwestError(e.to_string()))?;
        let base64_data = BASE64_STANDARD.encode(&bytes);
        Ok(FileContent {
            path: url_str.to_string(),
            text_content: None,
            image_data_base64: Some(base64_data),
            mime_type,
            lines_read: None, total_lines: None, truncated: None, error: None,
        })
    } else {
        let text_content = response.text().await.map_err(|e| AppError::ReqwestError(e.to_string()))?;
        let lines_count = text_content.lines().count();
        Ok(FileContent {
            path: url_str.to_string(),
            text_content: Some(text_content),
            image_data_base64: None,
            mime_type,
            lines_read: Some(lines_count), total_lines: Some(lines_count), truncated: Some(false), error: None,
        })
    }
}

#[tauri::command(async)]
#[instrument(skip(app_handle, config_state, audit_logger_state, params), fields(path = %params.path, is_url = %params.is_url))]
pub async fn read_file_command(
    app_handle: AppHandle,
    config_state: State<'_, Arc<StdRwLock<Config>>>,
    audit_logger_state: State<'_, Arc<crate::utils::audit_logger::AuditLogger>>,
    params: ReadFileParams,
) -> Result<FileContent, AppError> {
    audit_log(&audit_logger_state, "read_file", &serde_json::to_value(params)?).await;
    let config_guard = config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock error: {}",e)))?;

    if params.is_url {
        debug!(url = %params.path, "Reading from URL");
        // Consider using app_handle.http().fetch() if http plugin is configured for this URL's scope
        // For now, using reqwest directly as it was in the original code.
        let client = reqwest::Client::new();
        return read_file_from_url_internal(&client, params.path, &app_handle).await;
    }

    let path = validate_and_normalize_path(params.path, &config_guard, true, false)?;
    debug!(local_path = %path.display(), "Reading file from disk via tauri-plugin-fs");

    if !app_handle.fs_scope().is_allowed(&path) {
        warn!(path = %path.display(), "Path not allowed by tauri-plugin-fs scope for read_file");
        return Err(AppError::PathNotAllowed(format!("Path {} is not allowed by FS scope.", path.display())));
    }

    let mime_type = mime_guess::from_path(&path).first_or_octet_stream().to_string();

    if is_image_mime(&mime_type) {
        let bytes = app_handle.fs().read_binary(&path).await.map_err(|e| AppError::PluginError{ plugin: "fs".to_string(), message: e.to_string()})?;
        let base64_data = BASE64_STANDARD.encode(&bytes);
        Ok(FileContent {
            path: params.path.clone(),
            text_content: None, image_data_base64: Some(base64_data), mime_type,
            lines_read: None, total_lines: None, truncated: None, error: None,
        })
    } else {
        let full_content = app_handle.fs().read_text_file(&path).await.map_err(|e| AppError::PluginError{ plugin: "fs".to_string(), message: e.to_string()})?;
        let mut lines_iter = full_content.lines();
        let mut content_vec = Vec::new();
        let mut current_line_idx = 0;
        let mut total_lines_count = 0;
        let read_limit = params.length.unwrap_or(config_guard.file_read_line_limit);
        drop(config_guard);

        for line_str in lines_iter {
            total_lines_count += 1;
            if current_line_idx >= params.offset && content_vec.len() < read_limit {
                content_vec.push(line_str.to_string());
            }
            current_line_idx += 1;
            if content_vec.len() >= read_limit && (params.offset + content_vec.len()) < total_lines_count {
                break;
            }
        }
        let text_content_processed = content_vec.join("\n");
        let lines_read_count = content_vec.len();
        let is_truncated = params.offset > 0 || (lines_read_count == read_limit && (params.offset + lines_read_count) < total_lines_count);

        Ok(FileContent {
            path: params.path.clone(),
            text_content: Some(text_content_processed), image_data_base64: None, mime_type,
            lines_read: Some(lines_read_count), total_lines: Some(total_lines_count),
            truncated: Some(is_truncated), error: None,
        })
    }
}

#[tauri::command(async)]
#[instrument(skip(app_handle, config_state, audit_logger_state, params))]
pub async fn read_multiple_files_command(
    app_handle: AppHandle,
    config_state: State<'_, Arc<StdRwLock<Config>>>,
    audit_logger_state: State<'_, Arc<crate::utils::audit_logger::AuditLogger>>,
    params: ReadMultipleFilesParams,
) -> Result<ReadMultipleFilesResult, AppError> {
    audit_log(&audit_logger_state, "read_multiple_files", &serde_json::to_value(params)?).await;
    let config_guard = config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock error: {}",e)))?;

    let mut results = Vec::new();
    let http_client = reqwest::Client::new(); // Create one client for all URL reads

    for path_str_from_params in params.paths { // Iterate over borrowed strings
        let path_str = path_str_from_params.clone(); // Clone for ownership in ReadFileParams
        let is_url = path_str.starts_with("http://") || path_str.starts_with("https://");

        let read_params_for_single = ReadFileParams {
            path: path_str.clone(), // path_str is already owned here
            is_url,
            offset: 0,
            length: Some(config_guard.file_read_line_limit),
        };
        
        // We need to pass the audit_logger_state by reference if read_file_command also logs.
        // However, to avoid recursive audit logging for the same high-level operation,
        // we call an internal helper or re-implement the logic.
        // For simplicity here, re-implementing the core logic of single file read.
        let file_content_result = if is_url {
            read_file_from_url_internal(&http_client, &path_str, &app_handle).await
        } else {
            match validate_and_normalize_path(&path_str, &config_guard, true, false) {
                Ok(validated_path) => {
                    if !app_handle.fs_scope().is_allowed(&validated_path) {
                        Err(AppError::PathNotAllowed(format!("Read access to {} denied by FS scope.", validated_path.display())))
                    } else {
                        let mime_type = mime_guess::from_path(&validated_path).first_or_octet_stream().to_string();
                        if is_image_mime(&mime_type) {
                            app_handle.fs().read_binary(&validated_path).await
                                .map_err(|e| AppError::PluginError{plugin: "fs".to_string(), message: e.to_string()})
                                .map(|bytes| FileContent {
                                    path: path_str.clone(), text_content: None, image_data_base64: Some(BASE64_STANDARD.encode(&bytes)),
                                    mime_type, lines_read: None, total_lines: None, truncated: None, error: None,
                                })
                        } else {
                             app_handle.fs().read_text_file(&validated_path).await
                                .map_err(|e| AppError::PluginError{plugin: "fs".to_string(), message: e.to_string()})
                                .map(|text| {
                                    let line_count = text.lines().count(); // Simple line count for this context
                                    FileContent {
                                        path: path_str.clone(), text_content: Some(text), image_data_base64: None,
                                        mime_type, lines_read: Some(line_count), total_lines: Some(line_count), truncated: Some(false), error: None,
                                    }
                                })
                        }
                    }
                }
                Err(e) => Err(e),
            }
        };


        match file_content_result {
            Ok(content) => results.push(content),
            Err(e) => {
                warn!(path = %path_str, error = %e, "Failed to read one of multiple files");
                results.push(FileContent {
                    path: path_str.clone(), text_content: None, image_data_base64: None,
                    mime_type: "error/unknown".to_string(), lines_read: None, total_lines: None,
                    truncated: None, error: Some(e.to_string()),
                });
            }
        }
    }
    drop(config_guard);
    Ok(ReadMultipleFilesResult { results })
}


#[tauri::command(async)]
#[instrument(skip(app_handle, config_state, audit_logger_state, params), fields(path = %params.path, mode = ?params.mode))]
pub async fn write_file_command(
    app_handle: AppHandle,
    config_state: State<'_, Arc<StdRwLock<Config>>>,
    audit_logger_state: State<'_, Arc<crate::utils::audit_logger::AuditLogger>>,
    params: WriteFileParams,
) -> Result<FileOperationResult, AppError> {
    audit_log(&audit_logger_state, "write_file", &serde_json::to_value(params)?).await;
    let config_guard = config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock error: {}",e)))?;

    let path = validate_and_normalize_path(params.path, &config_guard, false, true)?;
    debug!(write_path = %path.display(), mode = ?params.mode, "Writing file via tauri-plugin-fs");

    let lines: Vec<&str> = params.content.lines().collect();
    if lines.len() > config_guard.file_write_line_limit {
        return Err(AppError::EditError(format!("Content exceeds line limit of {}. Received {} lines.", config_guard.file_write_line_limit, lines.len())));
    }

    let final_content = if params.mode == WriteMode::Append && app_handle.fs().exists(&path).await.unwrap_or(false) {
        let existing_content_str = app_handle.fs().read_text_file(&path).await.unwrap_or_default();
        let detected_ending = detect_line_ending(&existing_content_str);
        normalize_line_endings(params.content, detected_ending)
    } else {
        let system_ending = if cfg!(windows) { LineEndingStyle::CrLf } else { LineEndingStyle::Lf };
        normalize_line_endings(params.content, system_ending)
    };
    drop(config_guard);

    if !app_handle.fs_scope().is_allowed(&path) {
        return Err(AppError::PathNotAllowed(format!("Path {} not allowed for write by FS scope.", path.display())));
    }

    match params.mode {
        WriteMode::Rewrite => {
            app_handle.fs().write_text_file(&path, &final_content).await.map_err(|e| AppError::PluginError{ plugin: "fs".to_string(), message: e.to_string()})?;
        }
        WriteMode::Append => {
            let mut current_content = String::new();
            if app_handle.fs().exists(&path).await.unwrap_or(false) {
                current_content = app_handle.fs().read_text_file(&path).await.unwrap_or_default();
            }
            if !current_content.is_empty() && !current_content.ends_with('\n') && !current_content.ends_with("\r\n") {
                let detected_ending = detect_line_ending(current_content);
                current_content.push_str(detected_ending.as_str());
            }
            current_content.push_str(&final_content);
            app_handle.fs().write_text_file(&path, current_content).await.map_err(|e| AppError::PluginError{ plugin: "fs".to_string(), message: e.to_string()})?;
        }
    }
    Ok(FileOperationResult { success: true, path: params.path.clone(), message: format!("Successfully {} content to file.", if params.mode == WriteMode::Append {"appended"} else {"wrote"})})
}

#[tauri::command(async)]
#[instrument(skip(app_handle, config_state, audit_logger_state, params), fields(path = %params.path))]
pub async fn create_directory_command(
    app_handle: AppHandle,
    config_state: State<'_, Arc<StdRwLock<Config>>>,
    audit_logger_state: State<'_, Arc<crate::utils::audit_logger::AuditLogger>>,
    params: CreateDirectoryParams,
) -> Result<FileOperationResult, AppError> {
    audit_log(&audit_logger_state, "create_directory", &serde_json::to_value(params)?).await;
    let config_guard = config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock error: {}",e)))?;
    let path = validate_and_normalize_path(params.path, &config_guard, false, true)?;
    drop(config_guard);
    debug!(create_dir_path = %path.display(), "Creating directory via tauri-plugin-fs");

    if !app_handle.fs_scope().is_allowed(&path) {
        return Err(AppError::PathNotAllowed(format!("Path {} not allowed for dir creation by FS scope.", path.display())));
    }
    app_handle.fs().create_dir(&path, true).await.map_err(|e| AppError::PluginError{ plugin: "fs".to_string(), message: e.to_string()})?;
    Ok(FileOperationResult { success: true, path: params.path.clone(), message: "Directory created successfully.".to_string() })
}

#[tauri::command(async)]
#[instrument(skip(app_handle, config_state, audit_logger_state, params), fields(path = %params.path))]
pub async fn list_directory_command(
    app_handle: AppHandle,
    config_state: State<'_, Arc<StdRwLock<Config>>>,
    audit_logger_state: State<'_, Arc<crate::utils::audit_logger::AuditLogger>>,
    params: ListDirectoryParams,
) -> Result<ListDirectoryResult, AppError> {
    audit_log(&audit_logger_state, "list_directory", &serde_json::to_value(params)?).await;
    let config_guard = config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock error: {}",e)))?;
    let path = validate_and_normalize_path(params.path, &config_guard, true, false)?;
    drop(config_guard);
    debug!(list_dir_path = %path.display(), "Listing directory via tauri-plugin-fs");

    if !app_handle.fs_scope().is_allowed(&path) {
        return Err(AppError::PathNotAllowed(format!("Path {} not allowed for listing by FS scope.", path.display())));
    }
    let entries_from_plugin = app_handle.fs().read_dir(&path, false).await.map_err(|e| AppError::PluginError{ plugin: "fs".to_string(), message: e.to_string()})?;
    let mut entries = Vec::new();
    for entry_data in entries_from_plugin {
        let name = entry_data.name.unwrap_or_else(|| "unknown_entry".to_string());
        let prefix = if entry_data.is_dir { "[DIR] " } else { "[FILE]" };
        entries.push(format!("{} {}", prefix, name));
    }
    entries.sort();
    Ok(ListDirectoryResult { path: params.path.clone(), entries })
}

#[tauri::command(async)]
#[instrument(skip(app_handle, config_state, audit_logger_state, params), fields(source = %params.source, dest = %params.destination))]
pub async fn move_file_command(
    app_handle: AppHandle,
    config_state: State<'_, Arc<StdRwLock<Config>>>,
    audit_logger_state: State<'_, Arc<crate::utils::audit_logger::AuditLogger>>,
    params: MoveFileParams,
) -> Result<FileOperationResult, AppError> {
    audit_log(&audit_logger_state, "move_file", &serde_json::to_value(params)?).await;
    let config_guard = config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock error: {}",e)))?;
    let source_path = validate_and_normalize_path(params.source, &config_guard, true, false)?;
    let dest_path = validate_and_normalize_path(params.destination, &config_guard, false, true)?;
    drop(config_guard);
    debug!(move_source = %source_path.display(), move_dest = %dest_path.display(), "Moving file/directory via tauri-plugin-fs");

    if !app_handle.fs_scope().is_allowed(&source_path) || !app_handle.fs_scope().is_allowed(&dest_path.parent().unwrap_or(&dest_path)) {
         return Err(AppError::PathNotAllowed(format!("Source or destination parent for move not allowed by FS scope. Source: {}, Dest Parent: {}", source_path.display(), dest_path.parent().unwrap_or(&dest_path).display())));
    }
    app_handle.fs().rename_file(&source_path, &dest_path).await.map_err(|e| AppError::PluginError{ plugin: "fs".to_string(), message: e.to_string()})?;
    Ok(FileOperationResult { success: true, path: params.destination.clone(), message: format!("Successfully moved {} to {}.", params.source, params.destination) })
}

#[tauri::command(async)]
#[instrument(skip(app_handle, config_state, audit_logger_state, params), fields(path = %params.path))]
pub async fn get_file_info_command(
    app_handle: AppHandle,
    config_state: State<'_, Arc<StdRwLock<Config>>>,
    audit_logger_state: State<'_, Arc<crate::utils::audit_logger::AuditLogger>>,
    params: GetFileInfoParams,
) -> Result<FileInfoResult, AppError> {
    audit_log(&audit_logger_state, "get_file_info", &serde_json::to_value(params)?).await;
    let config_guard = config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock error: {}",e)))?;
    let path = validate_and_normalize_path(params.path, &config_guard, true, false)?;
    drop(config_guard);
    debug!(info_path = %path.display(), "Getting file info via tauri-plugin-fs");

    if !app_handle.fs_scope().is_allowed(&path) {
        return Err(AppError::PathNotAllowed(format!("Path {} not allowed for info by FS scope.", path.display())));
    }
    let metadata = app_handle.fs().metadata(&path).await.map_err(|e| AppError::PluginError{ plugin: "fs".to_string(), message: e.to_string()})?;
    let to_iso = |st_opt: Option<std::time::SystemTime>| st_opt.map(chrono::DateTime::<chrono::Utc>::from).map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
    
    let std_metadata = std::fs::metadata(&path)?;
    let permissions_octal = if cfg!(unix) {
        use std::os::unix::fs::PermissionsExt;
        Some(format!("{:03o}", std_metadata.permissions().mode() & 0o777))
    } else { None };

    Ok(FileInfoResult {
        path: params.path.clone(), size: metadata.len(), is_dir: metadata.is_dir(), is_file: metadata.is_file(),
        modified_iso: to_iso(metadata.modified), created_iso: to_iso(metadata.created), accessed_iso: to_iso(metadata.accessed),
        permissions_octal,
    })
}


async fn search_files_recursive_internal(
    app_handle: &AppHandle,
    dir_to_search: std::path::PathBuf,
    pattern_lower: &str,
    matches: &mut Vec<String>,
    current_depth: usize,
    max_depth: usize,
    files_root_for_relative_path: &Path,
    config_guard: &RwLockReadGuard<'_, Config>,
) -> Result<(), AppError> {
    if current_depth > max_depth { return Ok(()); }

    if !app_handle.fs_scope().is_allowed(&dir_to_search) {
        warn!(path = %dir_to_search.display(), "Search skipped: path not allowed by FS scope.");
        return Ok(());
    }
    if validate_and_normalize_path(dir_to_search.to_str().unwrap_or_default(), config_guard, true, false).is_err() {
        warn!(path = %dir_to_search.display(), "Search skipped: path not allowed by config.");
        return Ok(());
    }

    let dir_entries = match app_handle.fs().read_dir(&dir_to_search, false).await {
        Ok(entries) => entries,
        Err(e) => {
            warn!(path = %dir_to_search.display(), error = %e, "Could not read directory during search_files");
            return Ok(());
        }
    };

    for entry_data in dir_entries {
        let entry_name = entry_data.name.as_ref().map_or_else(String::new, |n| n.to_lowercase());
        let full_path = entry_data.path.clone();

        if entry_name.contains(pattern_lower) {
            if let Ok(relative_path) = full_path.strip_prefix(files_root_for_relative_path) {
                 matches.push(relative_path.to_string_lossy().into_owned());
            } else {
                matches.push(full_path.to_string_lossy().into_owned());
            }
        }
        if entry_data.is_dir && current_depth < max_depth {
            search_files_recursive_internal(app_handle, full_path, pattern_lower, matches, current_depth + 1, max_depth, files_root_for_relative_path, config_guard).await?;
        }
    }
    Ok(())
}

#[tauri::command(async)]
#[instrument(skip(app_handle, config_state, audit_logger_state, params), fields(path = %params.path, pattern = %params.pattern))]
pub async fn search_files_command(
    app_handle: AppHandle,
    config_state: State<'_, Arc<StdRwLock<Config>>>,
    audit_logger_state: State<'_, Arc<crate::utils::audit_logger::AuditLogger>>,
    params: SearchFilesParams,
) -> Result<SearchFilesResult, AppError> {
    audit_log(&audit_logger_state, "search_files", &serde_json::to_value(params)?).await;
    let config_guard = config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock error: {}",e)))?;

    let root_search_path = validate_and_normalize_path(params.path, &config_guard, true, false)?;
    let files_root_clone = config_guard.files_root.clone();
    debug!(search_root = %root_search_path.display(), pattern = %params.pattern, "Searching files by name (recursive with tauri-plugin-fs)");

    let search_operation = async {
        let mut matches = Vec::new();
        let pattern_lower = params.pattern.to_lowercase();
        if params.recursive {
            search_files_recursive_internal(&app_handle, root_search_path.clone(), &pattern_lower, &mut matches, 0, params.max_depth, &files_root_clone, &config_guard).await?;
        } else {
            if !app_handle.fs_scope().is_allowed(&root_search_path) || validate_and_normalize_path(root_search_path.to_str().unwrap_or_default(), &config_guard, true, false).is_err() {
                 warn!(path = %root_search_path.display(), "Search skipped: path not allowed by scope or config.");
                 return Ok(matches);
            }
            let dir_entries = app_handle.fs().read_dir(&root_search_path, false).await.map_err(|e| AppError::PluginError { plugin: "fs".to_string(), message: e.to_string() })?;
            for entry_data in dir_entries {
                let entry_name = entry_data.name.as_ref().map_or_else(String::new, |n| n.to_lowercase());
                 if entry_name.contains(&pattern_lower) {
                    if let Ok(relative_path) = entry_data.path.strip_prefix(&files_root_clone) {
                         matches.push(relative_path.to_string_lossy().into_owned());
                    } else { matches.push(entry_data.path.to_string_lossy().into_owned()); }
                }
            }
        }
        matches.sort();
        Ok(matches)
    };
    
    let config_guard_clone_for_timeout = config_guard.clone(); // Clone the guard for use inside timeout
    drop(config_guard); // Drop original guard before await

    let timeout_duration = Duration::from_millis(params.timeout_ms.unwrap_or(FILE_SEARCH_TIMEOUT_MS));
    match timeout(timeout_duration, search_operation).await {
        Ok(Ok(matches)) => Ok(SearchFilesResult { path: params.path.clone(), pattern: params.pattern.clone(), matches, timed_out: false }),
        Ok(Err(e)) => Err(e),
        Err(_) => {
            warn!("search_files timed out for path: {}, pattern: {}", params.path, params.pattern);
            Ok(SearchFilesResult { path: params.path.clone(), pattern: params.pattern.clone(), matches: vec![], timed_out: true })
        }
    }
}