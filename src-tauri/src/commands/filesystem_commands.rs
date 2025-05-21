use crate::config::Config;
use crate::error::AppError;
use crate::utils::path_utils::validate_and_normalize_path;
use crate::utils::line_ending_handler::{detect_line_ending, normalize_line_endings, LineEndingStyle};
use crate::utils::audit_logger::audit_log; // For logging command calls

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, RwLock as StdRwLock};
use tauri::{AppHandle, Manager, State};
use tokio::fs;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{debug, instrument, warn, error};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};

use tauri_plugin_fs::DirEntry; // For search_files

// --- New Request Structs for complex commands ---
#[derive(Debug, Deserialize, Serialize)] // Added Serialize for audit log
pub struct ReadMultipleFilesParams {
    pub paths: Vec<String>,
    // Maybe add option for how to handle errors: fail all or return partial with errors
}

#[derive(Debug, Deserialize, Serialize)] // Added Serialize for audit log
pub struct SearchFilesParams {
    pub path: String, // Root path for search
    pub pattern: String, // Case-insensitive substring to search for in file/directory names
    #[serde(rename = "timeoutMs")]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub recursive: bool, // Add recursive option
    #[serde(default = "default_search_max_depth")]
    pub max_depth: usize, // Max depth for recursive search
}
fn default_search_max_depth() -> usize { 10 } // Default max depth

// --- New Response Structs for complex commands ---
#[derive(Debug, Serialize)]
pub struct ReadMultipleFilesResult {
    pub results: Vec<FileContent>, // Each FileContent might have an error field
}

#[derive(Debug, Serialize)]
pub struct SearchFilesResult {
    pub path: String,
    pub pattern: String,
    pub matches: Vec<String>, // Paths relative to the initial search path or FILES_ROOT
    pub timed_out: bool,
}

const FILE_SEARCH_TIMEOUT_MS: u64 = 30000; // Default timeout for search_files

// --- read_multiple_files_command ---
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

    for path_str in params.paths {
        // Determine if it's a URL based on simple prefix check for this multi-read context
        // A more robust solution might require an explicit flag per path.
        let is_url = path_str.starts_with("http://") || path_str.starts_with("https://");

        let read_params = ReadFileParams { // Use the existing single ReadFileParams struct
            path: path_str.clone(),
            is_url,
            offset: 0,
            length: Some(config_guard.file_read_line_limit), // Use default limit for each file
        };

        let file_content_result = if is_url {
            read_file_from_url_internal(&http_client, path_str, &app_handle).await
        } else {
            // Re-implementing simplified local read logic here to avoid circular command calls
            // and to directly use app_handle.fs() with proper error wrapping.
            match validate_and_normalize_path(path_str, &config_guard, true, false) {
                Ok(validated_path) => {
                    if !app_handle.fs_scope().is_allowed(&validated_path) {
                        Err(AppError::PathNotAllowed(format!("Read access to {} denied by FS scope.", validated_path.display())))
                    } else {
                        let mime_type = mime_guess::from_path(&validated_path).first_or_octet_stream().to_string();
                        if is_image_mime(&mime_type) {
                            app_handle.fs().read_binary_file(&validated_path).await
                                .map_err(|e| AppError::PluginError{plugin: "fs".to_string(), message: e.to_string()})
                                .map(|bytes| FileContent {
                                    path: path_str.clone(),
                                    text_content: None,
                                    image_data_base64: Some(BASE64_STANDARD.encode(&bytes)),
                                    mime_type,
                                    lines_read: None, total_lines: None, truncated: None, error: None,
                                })
                        } else {
                            app_handle.fs().read_text_file(&validated_path).await
                                .map_err(|e| AppError::PluginError{plugin: "fs".to_string(), message: e.to_string()})
                                .map(|text| {
                                    let line_count = text.lines().count();
                                    FileContent {
                                        path: path_str.clone(),
                                        text_content: Some(text),
                                        image_data_base64: None,
                                        mime_type,
                                        lines_read: Some(line_count), total_lines: Some(line_count), truncated: Some(false), error: None,
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
                    path: path_str.clone(),
                    text_content: None, image_data_base64: None,
                    mime_type: "error/unknown".to_string(),
                    lines_read: None, total_lines: None, truncated: None,
                    error: Some(e.to_string()),
                });
            }
        }
    }
    drop(config_guard);
    Ok(ReadMultipleFilesResult { results })
}


// --- search_files_command ---
async fn search_files_recursive(
    app_handle: &AppHandle,
    dir_to_search: PathBuf,
    pattern_lower: &str,
    matches: &mut Vec<String>,
    current_depth: usize,
    max_depth: usize,
    files_root_for_relative_path: &Path, // To make paths relative
    config_guard: &StdRwLockReadGuard<'_, Config>, // Pass the guard for validation
) -> Result<(), AppError> {
    if current_depth > max_depth {
        return Ok(());
    }

    if !app_handle.fs_scope().is_allowed(&dir_to_search) {
        warn!(path = %dir_to_search.display(), "Search skipped: path not allowed by FS scope.");
        return Ok(()); // Skip directories not allowed by scope
    }
     // Also validate against config.allowed_directories
    if validate_and_normalize_path(dir_to_search.to_str().unwrap_or_default(), config_guard, true, false).is_err() {
        warn!(path = %dir_to_search.display(), "Search skipped: path not allowed by config.");
        return Ok(());
    }


    let dir_entries = match app_handle.fs().read_dir(&dir_to_search, false).await { // recursive = false for one level
        Ok(entries) => entries,
        Err(e) => {
            warn!(path = %dir_to_search.display(), error = %e, "Failed to read directory during search_files");
            return Ok(()); // Skip problematic directories
        }
    };

    for entry_data in dir_entries {
        let entry_name = entry_data.name.as_ref().map_or_else(String::new, |n| n.to_lowercase());
        let full_path = entry_data.path.clone(); // This should be absolute from plugin

        if entry_name.contains(pattern_lower) {
            // Make path relative to the original search root or FILES_ROOT
            if let Ok(relative_path) = full_path.strip_prefix(files_root_for_relative_path) {
                 matches.push(relative_path.to_string_lossy().into_owned());
            } else {
                matches.push(full_path.to_string_lossy().into_owned()); // Fallback to absolute
            }
        }

        if entry_data.is_dir && current_depth < max_depth { // Check max_depth before recursing
            search_files_recursive(app_handle, full_path, pattern_lower, matches, current_depth + 1, max_depth, files_root_for_relative_path, config_guard).await?;
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
    let files_root_clone = config_guard.files_root.clone(); // For making paths relative
    debug!(search_root = %root_search_path.display(), pattern = %params.pattern, "Searching files by name (recursive with tauri-plugin-fs)");

    let search_operation = async {
        let mut matches = Vec::new();
        let pattern_lower = params.pattern.to_lowercase();
        if params.recursive {
            search_files_recursive(&app_handle, root_search_path.clone(), &pattern_lower, &mut matches, 0, params.max_depth, &files_root_clone, &config_guard).await?;
        } else {
            // Non-recursive: only search in the top-level directory
            if !app_handle.fs_scope().is_allowed(&root_search_path) {
                 warn!(path = %root_search_path.display(), "Search skipped: path not allowed by FS scope.");
                 return Ok(matches);
            }
            if validate_and_normalize_path(root_search_path.to_str().unwrap_or_default(), &config_guard, true, false).is_err() {
                warn!(path = %root_search_path.display(), "Search skipped: path not allowed by config.");
                return Ok(matches);
            }

            let dir_entries = app_handle.fs().read_dir(&root_search_path, false).await
                .map_err(|e| AppError::PluginError { plugin: "fs".to_string(), message: e.to_string() })?;
            for entry_data in dir_entries {
                let entry_name = entry_data.name.as_ref().map_or_else(String::new, |n| n.to_lowercase());
                 if entry_name.contains(&pattern_lower) {
                    if let Ok(relative_path) = entry_data.path.strip_prefix(&files_root_clone) {
                         matches.push(relative_path.to_string_lossy().into_owned());
                    } else {
                        matches.push(entry_data.path.to_string_lossy().into_owned());
                    }
                }
            }
        }
        matches.sort();
        Ok(matches)
    };
    drop(config_guard); // Release lock before await on timeout

    let timeout_duration = Duration::from_millis(params.timeout_ms.unwrap_or(FILE_SEARCH_TIMEOUT_MS));
    match timeout(timeout_duration, search_operation).await {
        Ok(Ok(matches)) => Ok(SearchFilesResult {
            path: params.path.clone(),
            pattern: params.pattern.clone(),
            matches,
            timed_out: false,
        }),
        Ok(Err(e)) => Err(e), // Propagate AppError from search_operation
        Err(_) => { // Timeout error
            warn!("search_files timed out for path: {}, pattern: {}", params.path, params.pattern);
            Ok(SearchFilesResult {
                path: params.path.clone(),
                pattern: params.pattern.clone(),
                matches: vec![],
                timed_out: true,
            })
        }
    }
}

// --- Request Structs ---
#[derive(Debug, Deserialize)]
pub struct ReadFileParams {
    pub path: String,
    #[serde(default)]
    pub is_url: bool,
    #[serde(default)]
    pub offset: usize,
    pub length: Option<usize>,
}

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
pub struct CreateDirectoryParams {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct ListDirectoryParams {
    pub path: String,
}

#[derive(Debug, Deserialize)]
pub struct MoveFileParams {
    pub source: String,
    pub destination: String,
}

#[derive(Debug, Deserialize)]
pub struct GetFileInfoParams {
    pub path: String,
}

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
    pub error: Option<String>, // For multi-file reads where some might fail
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
    pub entries: Vec<String>, // Format: "[DIR] name" or "[FILE] name"
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

const URL_FETCH_TIMEOUT_MS: u64 = 30000;

fn is_image_mime(mime_type: &str) -> bool {
    mime_type.starts_with("image/") &&
    (mime_type.ends_with("/png") || mime_type.ends_with("/jpeg") || mime_type.ends_with("/gif") || mime_type.ends_with("/webp"))
}


async fn read_file_from_url_internal(
    http_client: &reqwest::Client,
    url_str: &str,
    _app_handle: &AppHandle // Keep for potential future use with http plugin directly
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
            response.error_for_status().map_err(|e| e.to_string()).unwrap_or_else(|e| format!("HTTP error: {}", e))
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
            lines_read: None,
            total_lines: None,
            truncated: None,
            error: None,
        })
    } else {
        let text_content = response.text().await.map_err(|e| AppError::ReqwestError(e.to_string()))?;
        let lines_count = text_content.lines().count();
        Ok(FileContent {
            path: url_str.to_string(),
            text_content: Some(text_content),
            image_data_base64: None,
            mime_type,
            lines_read: Some(lines_count),
            total_lines: Some(lines_count),
            truncated: Some(false),
            error: None,
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
    audit_log(&audit_logger_state, "read_file", &serde_json::to_value(&params)?).await;
    let config_guard = config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock error: {}",e)))?;

    if params.is_url {
        // For URL reading, we don't use tauri-plugin-fs scopes directly but rely on http plugin or reqwest.
        // The http plugin's scope is defined in tauri.conf.json.
        // If using reqwest directly, ensure network requests are intended.
        debug!(url = %params.path, "Reading from URL");
        // Using reqwest client directly for now as it's already a dependency.
        // Could be refactored to use tauri_plugin_http::fetch if preferred.
        let client = reqwest::Client::new();
        return read_file_from_url_internal(&client, &params.path, &app_handle).await;
    }

    let path = validate_and_normalize_path(&params.path, &config_guard, true, false)?;
    debug!(local_path = %path.display(), "Reading file from disk via tauri-plugin-fs");

    // Use tauri-plugin-fs for local file operations
    let fs_scope = app_handle.fs_scope();
    if !fs_scope.is_allowed(&path) {
         warn!(path = %path.display(), "Path not allowed by tauri-plugin-fs scope for read_file");
        return Err(AppError::PathNotAllowed(format!(
            "Path {} is not allowed by the application's file system scope.",
            path.display()
        )));
    }

    let mime_type = mime_guess::from_path(&path).first_or_octet_stream().to_string();

    if is_image_mime(&mime_type) {
        let bytes = app_handle.fs().read_binary_file(&path).await.map_err(|e| AppError::PluginError{ plugin: "fs".to_string(), message: e.to_string()})?;
        let base64_data = BASE64_STANDARD.encode(&bytes);
        Ok(FileContent {
            path: params.path.clone(),
            text_content: None,
            image_data_base64: Some(base64_data),
            mime_type,
            lines_read: None, total_lines: None, truncated: None, error: None,
        })
    } else {
        // For text files, tauri-plugin-fs read_text_file reads the whole file.
        // Manual line-by-line reading with offset/length needs custom implementation on top.
        let full_content = app_handle.fs().read_text_file(&path).await.map_err(|e| AppError::PluginError{ plugin: "fs".to_string(), message: e.to_string()})?;
        let mut lines_iter = full_content.lines();

        let mut content_vec = Vec::new();
        let mut current_line_idx = 0;
        let mut total_lines_count = 0;

        let read_limit = params.length.unwrap_or(config_guard.file_read_line_limit);

        for line_str in lines_iter {
            total_lines_count += 1;
            if current_line_idx >= params.offset && content_vec.len() < read_limit {
                content_vec.push(line_str.to_string());
            }
            current_line_idx += 1;
            if content_vec.len() >= read_limit && (params.offset + content_vec.len()) < total_lines_count {
                // Optimization: if we hit the read limit and there are more lines than what we've processed
                // (offset + read_limit) vs total_lines_count, we can stop early.
                // This requires knowing total_lines_count, which we get by iterating all lines.
                // If performance is critical for huge files, a true streaming read from disk is needed.
                // For now, this simulates it on the already read full_content.
                break;
            }
        }

        let text_content_processed = content_vec.join("\n");
        let lines_read_count = content_vec.len();
        let is_truncated = params.offset > 0 || (lines_read_count == read_limit && (params.offset + lines_read_count) < total_lines_count);

        Ok(FileContent {
            path: params.path.clone(),
            text_content: Some(text_content_processed),
            image_data_base64: None,
            mime_type,
            lines_read: Some(lines_read_count),
            total_lines: Some(total_lines_count), // This is the actual total lines in the file
            truncated: Some(is_truncated),
            error: None,
        })
    }
}

#[tauri::command(async)]
#[instrument(skip(app_handle, config_state, audit_logger_state, params), fields(path = %params.path, mode = ?params.mode))]
pub async fn write_file_command(
    app_handle: AppHandle,
    config_state: State<'_, Arc<StdRwLock<Config>>>,
    audit_logger_state: State<'_, Arc<crate::utils::audit_logger::AuditLogger>>,
    params: WriteFileParams,
) -> Result<FileOperationResult, AppError> {
    audit_log(&audit_logger_state, "write_file", &serde_json::to_value(&params)?).await;
    let config_guard = config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock error: {}",e)))?;

    let path = validate_and_normalize_path(&params.path, &config_guard, false, true)?; // check_existence=false, for_write=true
    debug!(write_path = %path.display(), mode = ?params.mode, "Writing file via tauri-plugin-fs");

    let lines: Vec<&str> = params.content.lines().collect();
    if lines.len() > config_guard.file_write_line_limit {
        return Err(AppError::EditError(format!(
            "Content exceeds line limit of {}. Received {} lines. Please send content in smaller chunks.",
            config_guard.file_write_line_limit,
            lines.len()
        )));
    }

    // Line ending normalization
    let final_content = if params.mode == WriteMode::Append && path.exists() {
        // Try to read existing content to detect line endings for append mode
        let existing_content_str = app_handle.fs().read_text_file(&path).await.unwrap_or_default();
        let detected_ending = detect_line_ending(&existing_content_str);
        normalize_line_endings(&params.content, detected_ending)
    } else {
        let system_ending = if cfg!(windows) { LineEndingStyle::CrLf } else { LineEndingStyle::Lf };
        normalize_line_endings(&params.content, system_ending)
    };
    drop(config_guard);


    let fs_scope = app_handle.fs_scope();
    if !fs_scope.is_allowed(&path) {
        warn!(path = %path.display(), "Path not allowed by tauri-plugin-fs scope for write_file");
        return Err(AppError::PathNotAllowed(format!(
            "Path {} is not allowed for writing by the application's file system scope.",
            path.display()
        )));
    }

    match params.mode {
        WriteMode::Rewrite => {
            app_handle.fs().write_text_file(&path, &final_content).await.map_err(|e| AppError::PluginError{ plugin: "fs".to_string(), message: e.to_string()})?;
        }
        WriteMode::Append => {
            // tauri-plugin-fs doesn't have a direct append_text_file.
            // We need to read, append, then write. This is not atomic.
            let mut current_content = String::new();
            if path.exists() { // Check existence with std::path::Path
                current_content = app_handle.fs().read_text_file(&path).await.unwrap_or_default();
            }
            // Ensure a newline if current content doesn't end with one and new content is being added
            if !current_content.is_empty() && !current_content.ends_with('\n') && !current_content.ends_with("\r\n") {
                let detected_ending = detect_line_ending(&current_content);
                current_content.push_str(detected_ending.as_str());
            }
            current_content.push_str(&final_content);
            app_handle.fs().write_text_file(&path, &current_content).await.map_err(|e| AppError::PluginError{ plugin: "fs".to_string(), message: e.to_string()})?;
        }
    }

    Ok(FileOperationResult {
        success: true,
        path: params.path.clone(),
        message: format!("Successfully {} content to file.", if params.mode == WriteMode::Append {"appended"} else {"wrote"}),
    })
}

#[tauri::command(async)]
#[instrument(skip(app_handle, config_state, audit_logger_state, params), fields(path = %params.path))]
pub async fn create_directory_command(
    app_handle: AppHandle,
    config_state: State<'_, Arc<StdRwLock<Config>>>,
    audit_logger_state: State<'_, Arc<crate::utils::audit_logger::AuditLogger>>,
    params: CreateDirectoryParams,
) -> Result<FileOperationResult, AppError> {
    audit_log(&audit_logger_state, "create_directory", &serde_json::to_value(&params)?).await;
    let config_guard = config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock error: {}",e)))?;
    let path = validate_and_normalize_path(&params.path, &config_guard, false, true)?; // check_existence=false, for_write=true (to validate parent)
    drop(config_guard);
    debug!(create_dir_path = %path.display(), "Creating directory via tauri-plugin-fs");

    let fs_scope = app_handle.fs_scope();
    if !fs_scope.is_allowed(&path) { // The scope should allow the target directory itself for creation
        warn!(path = %path.display(), "Path not allowed by tauri-plugin-fs scope for create_directory");
        return Err(AppError::PathNotAllowed(format!(
            "Path {} is not allowed for directory creation by the application's file system scope.",
            path.display()
        )));
    }

    app_handle.fs().create_dir(&path, true).await.map_err(|e| AppError::PluginError{ plugin: "fs".to_string(), message: e.to_string()})?; // recursive = true

    Ok(FileOperationResult {
        success: true,
        path: params.path.clone(),
        message: "Directory created successfully.".to_string(),
    })
}

#[tauri::command(async)]
#[instrument(skip(app_handle, config_state, audit_logger_state, params), fields(path = %params.path))]
pub async fn list_directory_command(
    app_handle: AppHandle,
    config_state: State<'_, Arc<StdRwLock<Config>>>,
    audit_logger_state: State<'_, Arc<crate::utils::audit_logger::AuditLogger>>,
    params: ListDirectoryParams,
) -> Result<ListDirectoryResult, AppError> {
    audit_log(&audit_logger_state, "list_directory", &serde_json::to_value(&params)?).await;
    let config_guard = config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock error: {}",e)))?;
    let path = validate_and_normalize_path(&params.path, &config_guard, true, false)?; // must exist, not for write
    drop(config_guard);
    debug!(list_dir_path = %path.display(), "Listing directory via tauri-plugin-fs");

    let fs_scope = app_handle.fs_scope();
    if !fs_scope.is_allowed(&path) {
        warn!(path = %path.display(), "Path not allowed by tauri-plugin-fs scope for list_directory");
        return Err(AppError::PathNotAllowed(format!(
            "Path {} is not allowed for listing by the application's file system scope.",
            path.display()
        )));
    }

    let entries_from_plugin = app_handle.fs().read_dir(&path, false).await.map_err(|e| AppError::PluginError{ plugin: "fs".to_string(), message: e.to_string()})?; // recursive = false

    let mut entries = Vec::new();
    for entry_data in entries_from_plugin {
        let name = entry_data.name.unwrap_or_else(|| "unknown_entry".to_string());
        let prefix = if entry_data.is_dir { "[DIR] " } else { "[FILE]" };
        entries.push(format!("{} {}", prefix, name));
    }
    entries.sort();

    Ok(ListDirectoryResult {
        path: params.path.clone(),
        entries,
    })
}

#[tauri::command(async)]
#[instrument(skip(app_handle, config_state, audit_logger_state, params), fields(source = %params.source, dest = %params.destination))]
pub async fn move_file_command(
    app_handle: AppHandle,
    config_state: State<'_, Arc<StdRwLock<Config>>>,
    audit_logger_state: State<'_, Arc<crate::utils::audit_logger::AuditLogger>>,
    params: MoveFileParams,
) -> Result<FileOperationResult, AppError> {
    audit_log(&audit_logger_state, "move_file", &serde_json::to_value(&params)?).await;
    let config_guard = config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock error: {}",e)))?;
    let source_path = validate_and_normalize_path(&params.source, &config_guard, true, false)?; // source must exist
    let dest_path = validate_and_normalize_path(&params.destination, &config_guard, false, true)?; // dest parent must be valid for write
    drop(config_guard);
    debug!(move_source = %source_path.display(), move_dest = %dest_path.display(), "Moving file/directory via tauri-plugin-fs");

    let fs_scope = app_handle.fs_scope();
    if !fs_scope.is_allowed(&source_path) || !fs_scope.is_allowed(&dest_path) { // Simplified check, plugin might have more granular rename scope
         warn!(source = %source_path.display(), dest = %dest_path.display(), "Source or destination path not allowed by tauri-plugin-fs scope for move_file");
        return Err(AppError::PathNotAllowed(format!(
            "Source ({}) or destination ({}) path not allowed for move by the application's file system scope.",
            source_path.display(), dest_path.display()
        )));
    }

    app_handle.fs().rename_file(&source_path, &dest_path).await.map_err(|e| AppError::PluginError{ plugin: "fs".to_string(), message: e.to_string()})?;

    Ok(FileOperationResult {
        success: true,
        path: params.destination.clone(),
        message: format!("Successfully moved {} to {}.", params.source, params.destination),
    })
}

#[tauri::command(async)]
#[instrument(skip(app_handle, config_state, audit_logger_state, params), fields(path = %params.path))]
pub async fn get_file_info_command(
    app_handle: AppHandle,
    config_state: State<'_, Arc<StdRwLock<Config>>>,
    audit_logger_state: State<'_, Arc<crate::utils::audit_logger::AuditLogger>>,
    params: GetFileInfoParams,
) -> Result<FileInfoResult, AppError> {
    audit_log(&audit_logger_state, "get_file_info", &serde_json::to_value(&params)?).await;
    let config_guard = config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock error: {}",e)))?;
    let path = validate_and_normalize_path(&params.path, &config_guard, true, false)?; // must exist
    drop(config_guard);
    debug!(info_path = %path.display(), "Getting file info via tauri-plugin-fs");

    let fs_scope = app_handle.fs_scope();
    if !fs_scope.is_allowed(&path) {
        warn!(path = %path.display(), "Path not allowed by tauri-plugin-fs scope for get_file_info");
        return Err(AppError::PathNotAllowed(format!(
            "Path {} is not allowed for info by the application's file system scope.",
            path.display()
        )));
    }

    let metadata = app_handle.fs().metadata(&path).await.map_err(|e| AppError::PluginError{ plugin: "fs".to_string(), message: e.to_string()})?;

    let to_iso = |st_opt: Option<std::time::SystemTime>| {
        st_opt
            .map(chrono::DateTime::<chrono::Utc>::from)
            .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
    };
    
    // tauri_plugin_fs::Metadata doesn't directly expose raw mode for permissions.
    // We'll use std::fs for that part if needed, but it's platform-specific.
    // For simplicity, permissions_octal might be harder to get consistently via plugin.
    let std_metadata = std::fs::metadata(&path)?; // Fallback to std::fs for permissions
    let permissions_octal = if cfg!(unix) {
        use std::os::unix::fs::PermissionsExt;
        Some(format!("{:03o}", std_metadata.permissions().mode() & 0o777))
    } else {
        None // Windows permissions are more complex than simple octal.
    };


    Ok(FileInfoResult {
        path: params.path.clone(),
        size: metadata.len(),
        is_dir: metadata.is_dir(),
        is_file: metadata.is_file(),
        modified_iso: to_iso(metadata.modified), // Assuming tauri_plugin_fs::Metadata has these fields
        created_iso: to_iso(metadata.created),
        accessed_iso: to_iso(metadata.accessed),
        permissions_octal,
    })
}