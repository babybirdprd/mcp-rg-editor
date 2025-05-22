use crate::config::Config;
use crate::error::AppError;
use crate::mcp::handler::ToolDependencies;
use crate::utils::path_utils::validate_and_normalize_path;
use crate::utils::line_ending_handler::{detect_line_ending, normalize_line_endings, LineEndingStyle};

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::RwLockReadGuard;
use tauri_plugin_fs::{DirEntry, FilePath, FsExt, DirOptions, FileOptions, Metadata as FsMetadata};
use tracing::{debug, warn, instrument};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use tokio::time::{timeout, Duration};


// --- MCP Specific Parameter Structs ---
#[derive(Debug, Deserialize, Serialize)]
pub struct ReadFileParamsMCP {
    pub path: String,
    #[serde(default)]
    pub is_url: bool,
    #[serde(default)]
    pub offset: usize,
    pub length: Option<usize>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ReadMultipleFilesParamsMCP {
    pub paths: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct WriteFileParamsMCP {
    pub path: String,
    pub content: String,
    #[serde(default = "default_rewrite_mode_mcp")]
    pub mode: WriteModeMCP,
}
fn default_rewrite_mode_mcp() -> WriteModeMCP { WriteModeMCP::Rewrite }

#[derive(Debug, Deserialize, PartialEq, Eq, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WriteModeMCP { Rewrite, Append }

#[derive(Debug, Deserialize, Serialize)]
pub struct CreateDirectoryParamsMCP { pub path: String }
#[derive(Debug, Deserialize, Serialize)]
pub struct ListDirectoryParamsMCP { pub path: String }
#[derive(Debug, Deserialize, Serialize)]
pub struct MoveFileParamsMCP { pub source: String, pub destination: String }
#[derive(Debug, Deserialize, Serialize)]
pub struct GetFileInfoParamsMCP { pub path: String }

#[derive(Debug, Deserialize, Serialize)]
pub struct SearchFilesParamsMCP {
    pub path: String,
    pub pattern: String,
    #[serde(rename = "timeoutMs")]
    pub timeout_ms: Option<u64>,
    #[serde(default)]
    pub recursive: bool,
    #[serde(default = "default_search_max_depth_mcp")]
    pub max_depth: usize,
}
fn default_search_max_depth_mcp() -> usize { 10 }


// --- MCP Specific Result Structs ---
#[derive(Debug, Serialize)]
pub struct FileContentMCP {
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
pub struct ReadMultipleFilesResultMCP { pub results: Vec<FileContentMCP> }
#[derive(Debug, Serialize)]
pub struct FileOperationResultMCP { pub success: bool, pub path: String, pub message: String }
#[derive(Debug, Serialize)]
pub struct ListDirectoryResultMCP { pub path: String, pub entries: Vec<DirEntry> }

#[derive(Debug, Serialize)]
pub struct FileInfoResultMCP {
    pub path: String, pub size: u64, pub is_dir: bool, pub is_file: bool,
    #[serde(skip_serializing_if = "Option::is_none")] pub modified_iso: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub created_iso: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub accessed_iso: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub permissions_octal: Option<String>,
}
#[derive(Debug, Serialize)]
pub struct SearchFilesResultMCP { pub path: String, pub pattern: String, pub matches: Vec<String>, pub timed_out: bool }


const URL_FETCH_TIMEOUT_MS_MCP: u64 = 30000;
const FILE_SEARCH_TIMEOUT_MS_MCP: u64 = 30000;

fn is_image_mime_mcp(mime_type: &str) -> bool {
    mime_type.starts_with("image/") && (mime_type.ends_with("/png") || mime_type.ends_with("/jpeg") || mime_type.ends_with("/gif") || mime_type.ends_with("/webp"))
}

#[instrument(skip(http_client, _app_handle), fields(url = %url_str))]
async fn read_file_from_url_mcp_internal(
    http_client: &reqwest::Client,
    url_str: &str,
    _app_handle: &tauri::AppHandle,
) -> Result<FileContentMCP, AppError> {
    debug!("MCP Tool: Reading file from URL via reqwest");
    let response_res = timeout(Duration::from_millis(URL_FETCH_TIMEOUT_MS_MCP), http_client.get(url_str).send()).await;

    let response = match response_res {
        Ok(Ok(resp)) => resp,
        Ok(Err(e)) => return Err(AppError::ReqwestError(e.to_string())),
        Err(_) => return Err(AppError::TimeoutError(format!("URL fetch timed out: {}", url_str))),
    };

    let status = response.status();
    if !status.is_success() {
        let err_msg = response.text().await.unwrap_or_else(|_| "Unknown HTTP error".to_string());
        return Err(AppError::ReqwestError(format!("HTTP Error {}: {}", status, err_msg)));
    }

    let mime_type = response.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v|v.to_str().ok()).unwrap_or("application/octet-stream").split(';').next().unwrap_or_default().trim().to_string();
    if is_image_mime_mcp(&mime_type) {
        let bytes = response.bytes().await.map_err(|e|AppError::ReqwestError(e.to_string()))?;
        Ok(FileContentMCP { path: url_str.to_string(), text_content: None, image_data_base64: Some(BASE64_STANDARD.encode(&bytes)), mime_type, lines_read: None, total_lines: None, truncated: None, error: None })
    } else {
        let text = response.text().await.map_err(|e|AppError::ReqwestError(e.to_string()))?;
        let lines_count = text.lines().count();
        Ok(FileContentMCP { path: url_str.to_string(), text_content: Some(text), image_data_base64: None, mime_type, lines_read: Some(lines_count), total_lines: Some(lines_count), truncated: Some(false), error: None })
    }
}

#[instrument(skip(deps, params), fields(path = %params.path, is_url = %params.is_url))]
pub async fn mcp_read_file(deps: &ToolDependencies, params: ReadFileParamsMCP) -> Result<FileContentMCP, AppError> {
    let config_guard = deps.config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock: {}", e)))?;
    if params.is_url {
        let client = reqwest::Client::new();
        return read_file_from_url_mcp_internal(&client, &params.path, &deps.app_handle).await;
    }
    let path = validate_and_normalize_path(&params.path, &config_guard, true, false)?;
    if !deps.app_handle.fs_scope().is_allowed(&path) { return Err(AppError::PathNotAllowed(format!("FS scope disallows read: {}", path.display()))); }

    let mime_type = mime_guess::from_path(&path).first_or_octet_stream().to_string();
    if is_image_mime_mcp(&mime_type) {
        let bytes = deps.app_handle.read_binary_file(FilePath::Path(path)) // MODIFIED
            .map_err(|e| AppError::PluginError{ plugin:"fs".to_string(), message:e.to_string()})?;
        Ok(FileContentMCP { path: params.path, text_content: None, image_data_base64: Some(BASE64_STANDARD.encode(&bytes)), mime_type, lines_read: None, total_lines: None, truncated: None, error: None })
    } else {
        let full_content = deps.app_handle.read_text_file(FilePath::Path(path)) // MODIFIED
            .map_err(|e| AppError::PluginError{ plugin:"fs".to_string(), message:e.to_string()})?;
        let mut lines_iter = full_content.lines();
        let mut content_vec = Vec::new();
        let mut current_line_idx = 0;
        let mut total_lines_count = 0;
        let read_limit = params.length.unwrap_or(config_guard.file_read_line_limit);

        for line_str in lines_iter {
            total_lines_count += 1;
            if current_line_idx >= params.offset && content_vec.len() < read_limit { content_vec.push(line_str.to_string()); }
            current_line_idx += 1;
            if content_vec.len() >= read_limit && (params.offset + content_vec.len()) < total_lines_count { break; }
        }
        let text_processed = content_vec.join("\n");
        let lines_read = content_vec.len();
        let truncated = params.offset > 0 || (lines_read == read_limit && (params.offset + lines_read) < total_lines_count);
        Ok(FileContentMCP { path: params.path, text_content: Some(text_processed), image_data_base64: None, mime_type, lines_read: Some(lines_read), total_lines: Some(total_lines_count), truncated: Some(truncated), error: None })
    }
}

#[instrument(skip(deps, params), fields(path = %params.path, mode = ?params.mode))]
pub async fn mcp_write_file(deps: &ToolDependencies, params: WriteFileParamsMCP) -> Result<FileOperationResultMCP, AppError> {
    let config_guard = deps.config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock: {}", e)))?;
    let path = validate_and_normalize_path(&params.path, &config_guard, false, true)?;
    let lines: Vec<&str> = params.content.lines().collect();
    if lines.len() > config_guard.file_write_line_limit { return Err(AppError::EditError(format!("Content exceeds line limit {}. Received {}.", config_guard.file_write_line_limit, lines.len()))); }

    let final_content_str = if params.mode == WriteModeMCP::Append && deps.app_handle.exists(FilePath::Path(path.clone())).unwrap_or(false) { // MODIFIED
        let existing_content_str = deps.app_handle.read_text_file(FilePath::Path(path.clone())).unwrap_or_default(); // MODIFIED
        normalize_line_endings(&params.content, detect_line_ending(&existing_content_str))
    } else { normalize_line_endings(&params.content, if cfg!(windows) {LineEndingStyle::CrLf} else {LineEndingStyle::Lf}) };

    if !deps.app_handle.fs_scope().is_allowed(&path) { return Err(AppError::PathNotAllowed(format!("FS scope disallows write: {}", path.display()))); }

    let fs_options = FileOptions {
        append: Some(params.mode == WriteModeMCP::Append),
        base_dir: None,
    };

    deps.app_handle.write_text_file(FilePath::Path(path.clone()), final_content_str, Some(fs_options)) // MODIFIED
        .map_err(|e|AppError::PluginError{plugin:"fs".to_string(), message:e.to_string()})?;

    Ok(FileOperationResultMCP { success: true, path: params.path, message: format!("Successfully {} content.", if params.mode == WriteModeMCP::Append {"appended"} else {"wrote"})})
}

#[instrument(skip(deps, params), fields(path = %params.path))]
pub async fn mcp_create_directory(deps: &ToolDependencies, params: CreateDirectoryParamsMCP) -> Result<FileOperationResultMCP, AppError> {
    let config_guard = deps.config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock: {}", e)))?;
    let path = validate_and_normalize_path(&params.path, &config_guard, false, true)?;
    if !deps.app_handle.fs_scope().is_allowed(&path) { return Err(AppError::PathNotAllowed(format!("FS scope disallows dir creation: {}", path.display()))); }
    deps.app_handle.create_dir(FilePath::Path(path), Some(DirOptions { recursive: Some(true), base_dir: None })) // MODIFIED
        .map_err(|e|AppError::PluginError{plugin:"fs".into(), message:e.to_string()})?;
    Ok(FileOperationResultMCP { success: true, path: params.path, message: "Directory created.".to_string() })
}

#[instrument(skip(deps, params), fields(path = %params.path))]
pub async fn mcp_list_directory(deps: &ToolDependencies, params: ListDirectoryParamsMCP) -> Result<ListDirectoryResultMCP, AppError> {
    let config_guard = deps.config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock: {}", e)))?;
    let path = validate_and_normalize_path(&params.path, &config_guard, true, false)?;
    if !deps.app_handle.fs_scope().is_allowed(&path) { return Err(AppError::PathNotAllowed(format!("FS scope disallows list: {}", path.display()))); }
    let entries_data = deps.app_handle.read_dir(FilePath::Path(path), Some(DirOptions { recursive: Some(false), base_dir: None })) // MODIFIED
        .map_err(|e|AppError::PluginError{plugin:"fs".into(), message:e.to_string()})?;
    Ok(ListDirectoryResultMCP { path: params.path, entries: entries_data })
}

#[instrument(skip(deps, params), fields(source = %params.source, dest = %params.destination))]
pub async fn mcp_move_file(deps: &ToolDependencies, params: MoveFileParamsMCP) -> Result<FileOperationResultMCP, AppError> {
    let config_guard = deps.config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock: {}", e)))?;
    let source_path = validate_and_normalize_path(&params.source, &config_guard, true, false)?;
    let dest_path = validate_and_normalize_path(&params.destination, &config_guard, false, true)?;
    if !deps.app_handle.fs_scope().is_allowed(&source_path) || !deps.app_handle.fs_scope().is_allowed(&dest_path.parent().unwrap_or(&dest_path)) {
        return Err(AppError::PathNotAllowed(format!("FS scope disallows move from {} or to {}", source_path.display(), dest_path.parent().unwrap_or(&dest_path).display())));
    }
    deps.app_handle.rename_file(FilePath::Path(source_path), FilePath::Path(dest_path)) // MODIFIED (rename -> rename_file)
        .map_err(|e|AppError::PluginError{plugin:"fs".into(), message:e.to_string()})?;
    Ok(FileOperationResultMCP { success: true, path: params.destination, message: format!("Moved {} to {}.", params.source, params.destination) })
}

#[instrument(skip(deps, params), fields(path = %params.path))]
pub async fn mcp_get_file_info(deps: &ToolDependencies, params: GetFileInfoParamsMCP) -> Result<FileInfoResultMCP, AppError> {
    let config_guard = deps.config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock: {}", e)))?;
    let path = validate_and_normalize_path(&params.path, &config_guard, true, false)?;
    if !deps.app_handle.fs_scope().is_allowed(&path) { return Err(AppError::PathNotAllowed(format!("FS scope disallows info: {}", path.display()))); }

    let fs_metadata: FsMetadata = deps.app_handle.metadata(FilePath::Path(path.clone())) // MODIFIED
        .map_err(|e|AppError::PluginError{plugin:"fs".into(), message:e.to_string()})?;

    let to_iso = |st_opt_ms: Option<u128>| st_opt_ms.map(|st_ms| {
        let secs = (st_ms / 1000) as i64;
        let nanos = ((st_ms % 1000) * 1_000_000) as u32;
        chrono::DateTime::<chrono::Utc>::from_timestamp(secs, nanos)
            .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
            .unwrap_or_default()
    });

    let std_meta = std::fs::metadata(&path).map_err(|e| AppError::StdIoError(e.to_string()))?;
    let perms = {
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            Some(format!("{:03o}", std_meta.permissions().mode() & 0o777))
        }
        #[cfg(not(unix))] { None::<String> }
    };
    Ok(FileInfoResultMCP {
        path: params.path,
        size: fs_metadata.len,
        is_dir: fs_metadata.is_dir,
        is_file: fs_metadata.is_file,
        modified_iso: to_iso(fs_metadata.modified_at_ms),
        created_iso: to_iso(fs_metadata.created_at_ms),
        accessed_iso: to_iso(fs_metadata.accessed_at_ms),
        permissions_octal: perms
    })
}

#[instrument(skip(deps, params), fields(paths_count = %params.paths.len()))]
pub async fn mcp_read_multiple_files(deps: &ToolDependencies, params: ReadMultipleFilesParamsMCP) -> Result<ReadMultipleFilesResultMCP, AppError> {
    let config_guard = deps.config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock: {}", e)))?;
    let mut results = Vec::new();
    let http_client = reqwest::Client::new();

    for path_str_from_params in params.paths {
        let path_str = path_str_from_params.clone();
        let is_url = path_str.starts_with("http://") || path_str.starts_with("https://");

        let content_res = if is_url {
            read_file_from_url_mcp_internal(&http_client, &path_str, &deps.app_handle).await
        } else {
            match validate_and_normalize_path(&path_str, &config_guard, true, false) {
                Ok(val_path) => {
                    if !deps.app_handle.fs_scope().is_allowed(&val_path) { Err(AppError::PathNotAllowed(format!("FS scope disallows read: {}", val_path.display()))) }
                    else {
                        let mime = mime_guess::from_path(&val_path).first_or_octet_stream().to_string();
                        if is_image_mime_mcp(&mime) {
                            deps.app_handle.read_binary_file(FilePath::Path(val_path)) // MODIFIED
                                .map_err(|e|AppError::PluginError{plugin:"fs".to_string(), message:e.to_string()})
                                .map(|b| FileContentMCP{path:path_str.clone(), text_content:None, image_data_base64:Some(BASE64_STANDARD.encode(&b)), mime_type:mime, lines_read:None, total_lines:None, truncated:None, error:None})
                        } else {
                            deps.app_handle.read_text_file(FilePath::Path(val_path)) // MODIFIED
                                .map_err(|e|AppError::PluginError{plugin:"fs".to_string(), message:e.to_string()})
                                .map(|txt| { let lc=txt.lines().count(); FileContentMCP{path:path_str.clone(), text_content:Some(txt), image_data_base64:None, mime_type:mime, lines_read:Some(lc), total_lines:Some(lc), truncated:Some(false), error:None}})
                        }
                    }
                }
                Err(e) => Err(e),
            }
        };
        match content_res {
            Ok(c) => results.push(c),
            Err(e) => results.push(FileContentMCP{path:path_str.clone(), text_content:None, image_data_base64:None, mime_type:"error/unknown".into(), lines_read:None, total_lines:None, truncated:None, error:Some(e.to_string())}),
        }
    }
    Ok(ReadMultipleFilesResultMCP { results })
}

#[instrument(skip(app_handle, pattern_lower, matches, config_guard), fields(dir = %dir_to_search.display()))]
async fn search_files_recursive_mcp_internal(
    app_handle: &tauri::AppHandle,
    dir_to_search: PathBuf,
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

    let dir_entries_result = app_handle.read_dir(FilePath::Path(dir_to_search.clone()), Some(DirOptions { recursive: Some(false), base_dir: None })); // MODIFIED
    let dir_entries = match dir_entries_result {
        Ok(entries) => entries,
        Err(e) => {
            warn!(path = %dir_to_search.display(), error = %e, "Could not read directory during search_files");
            return Ok(()); // Don't fail the whole search for one unreadable dir
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
        if entry_data.is_dir.unwrap_or(false) && current_depth < max_depth {
            search_files_recursive_mcp_internal(app_handle, full_path, pattern_lower, matches, current_depth + 1, max_depth, files_root_for_relative_path, config_guard).await?;
        }
    }
    Ok(())
}

#[instrument(skip(deps, params), fields(path = %params.path, pattern = %params.pattern))]
pub async fn mcp_search_files(deps: &ToolDependencies, params: SearchFilesParamsMCP) -> Result<SearchFilesResultMCP, AppError> {
    let config_guard = deps.config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock: {}", e)))?;
    let root_search_path = validate_and_normalize_path(&params.path, &config_guard, true, false)?;
    let files_root_clone = config_guard.files_root.clone();

    let app_handle_clone = deps.app_handle.clone();
    let pattern_lower_clone = params.pattern.to_lowercase();
    let max_depth_clone = params.max_depth;
    let recursive_clone = params.recursive;

    let config_values_for_async = (
        config_guard.files_root.clone(),
    );
    drop(config_guard);


    let search_operation = async {
        let mut matches = Vec::new();

        if recursive_clone {
            let temp_config_guard_for_recursion = deps.config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock for recursion: {}", e)))?;
            search_files_recursive_mcp_internal(&app_handle_clone, root_search_path.clone(), &pattern_lower_clone, &mut matches, 0, max_depth_clone, &files_root_clone, &temp_config_guard_for_recursion).await?;
        } else {
            if !app_handle_clone.fs_scope().is_allowed(&root_search_path) {
                 let temp_config_guard_for_validation = deps.config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock for validation: {}", e)))?;
                 if validate_and_normalize_path(root_search_path.to_str().unwrap_or_default(), &temp_config_guard_for_validation, true, false).is_err() {
                    warn!(path = %root_search_path.display(), "Search skipped: path not allowed by scope or config.");
                    return Ok(matches);
                 }
            }
            let dir_entries = app_handle_clone.read_dir(FilePath::Path(root_search_path), Some(DirOptions { recursive: Some(false), base_dir: None })) // MODIFIED
                .map_err(|e| AppError::PluginError { plugin: "fs".to_string(), message: e.to_string() })?;
            for entry_data in dir_entries {
                let entry_name = entry_data.name.as_ref().map_or_else(String::new, |n| n.to_lowercase());
                 if entry_name.contains(&pattern_lower_clone) {
                    if let Ok(relative_path) = entry_data.path.strip_prefix(&files_root_clone) {
                         matches.push(relative_path.to_string_lossy().into_owned());
                    } else { matches.push(entry_data.path.to_string_lossy().into_owned()); }
                }
            }
        }
        matches.sort();
        Result::<Vec<String>, AppError>::Ok(matches)
    };

    match timeout(Duration::from_millis(params.timeout_ms.unwrap_or(FILE_SEARCH_TIMEOUT_MS_MCP)), search_operation).await {
        Ok(Ok(m)) => Ok(SearchFilesResultMCP { path: params.path, pattern: params.pattern, matches: m, timed_out: false }),
        Ok(Err(e)) => Err(e),
        Err(_) => Ok(SearchFilesResultMCP { path: params.path, pattern: params.pattern, matches: vec![], timed_out: true }),
    }
}