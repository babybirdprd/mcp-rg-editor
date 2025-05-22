// FILE: src-tauri/src/mcp/tool_impl/filesystem.rs
use crate::config::Config;
use crate::error::AppError;
use crate::mcp::handler::ToolDependencies;
use crate::utils::path_utils::validate_and_normalize_path;
use crate::utils::line_ending_handler::{detect_line_ending, normalize_line_endings, LineEndingStyle};

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf}; // Use PathBuf consistently
use std::sync::RwLockReadGuard;
use tauri::Manager;
use tauri_plugin_fs::FsExt; // Import the FsExt trait
use tracing::{debug, warn}; // Removed unused `error`
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use tokio::time::{timeout, Duration};


// --- MCP Specific Parameter Structs ---
#[derive(Debug, Deserialize)]
pub struct ReadFileParamsMCP {
    pub path: String,
    #[serde(default)]
    pub is_url: bool,
    #[serde(default)]
    pub offset: usize,
    pub length: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct ReadMultipleFilesParamsMCP {
    pub paths: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct WriteFileParamsMCP {
    pub path: String,
    pub content: String,
    #[serde(default = "default_rewrite_mode_mcp")]
    pub mode: WriteModeMCP,
}
fn default_rewrite_mode_mcp() -> WriteModeMCP { WriteModeMCP::Rewrite }

#[derive(Debug, Deserialize, PartialEq, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WriteModeMCP { Rewrite, Append }

#[derive(Debug, Deserialize)]
pub struct CreateDirectoryParamsMCP { pub path: String } // Corrected syntax
#[derive(Debug, Deserialize)]
pub struct ListDirectoryParamsMCP { pub path: String } // Corrected syntax
#[derive(Debug, Deserialize)]
pub struct MoveFileParamsMCP { pub source: String, pub destination: String } // Corrected syntax
#[derive(Debug, Deserialize)]
pub struct GetFileInfoParamsMCP { pub path: String } // Corrected syntax

#[derive(Debug, Deserialize)]
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
pub struct ReadMultipleFilesResultMCP { pub results: Vec<FileContentMCP> } // Corrected syntax
#[derive(Debug, Serialize)]
pub struct FileOperationResultMCP { pub success: bool, pub path: String, pub message: String } // Corrected syntax
#[derive(Debug, Serialize)]
pub struct ListDirectoryResultMCP { pub path: String, pub entries: Vec<String> } // Corrected syntax

#[derive(Debug, Serialize)]
pub struct FileInfoResultMCP {
    pub path: String, pub size: u64, pub is_dir: bool, pub is_file: bool,
    #[serde(skip_serializing_if = "Option::is_none")] pub modified_iso: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub created_iso: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub accessed_iso: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub permissions_octal: Option<String>,
}
#[derive(Debug, Serialize)]
pub struct SearchFilesResultMCP { pub path: String, pub pattern: String, pub matches: Vec<String>, pub timed_out: bool } // Corrected syntax


const URL_FETCH_TIMEOUT_MS_MCP: u64 = 30000;
const FILE_SEARCH_TIMEOUT_MS_MCP: u64 = 30000;

fn is_image_mime_mcp(mime_type: &str) -> bool {
    mime_type.starts_with("image/") && (mime_type.ends_with("/png") || mime_type.ends_with("/jpeg") || mime_type.ends_with("/gif") || mime_type.ends_with("/webp"))
}

async fn read_file_from_url_mcp_internal(http_client: &reqwest::Client, url_str: &str, _app_handle: &tauri::AppHandle) -> Result<FileContentMCP, AppError> {
    debug!(url = %url_str, "MCP Tool: Reading file from URL via reqwest");
    let response = match timeout(Duration::from_millis(URL_FETCH_TIMEOUT_MS_MCP), http_client.get(url_str).send()).await {
        Ok(Ok(resp)) => resp,
        Ok(Err(e)) => return Err(AppError::ReqwestError(e.to_string())),
        Err(_) => return Err(AppError::TimeoutError(format!("URL fetch timed out: {}", url_str))),
    };
    
    if !response.status().is_success() {
        let err_msg = response.text().await.unwrap_or_else(|_| "Unknown HTTP error".to_string());
        return Err(AppError::ReqwestError(format!("HTTP Error {}: {}", response.status(), err_msg)));
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

pub async fn mcp_read_file(deps: &ToolDependencies, params: ReadFileParamsMCP) -> Result<FileContentMCP, AppError> {
    let config_guard = deps.config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock: {}", e)))?;
    if params.is_url {
        let client = reqwest::Client::new(); // Consider making this part of ToolDependencies if used frequently
        return read_file_from_url_mcp_internal(&client, params.path, &deps.app_handle).await;
    }
    let path = validate_and_normalize_path(params.path, &config_guard, true, false)?;
    if !deps.app_handle.fs_scope().is_allowed(&path) { return Err(AppError::PathNotAllowed(format!("FS scope disallows read: {}", path.display()))); }
    
    let mime_type = mime_guess::from_path(&path).first_or_octet_stream().to_string();
    if is_image_mime_mcp(&mime_type) {
        let bytes = deps.app_handle.fs().read_binary(&path).await.map_err(|e| AppError::PluginError{plugin:"fs".into(), message:e.to_string()})?;
        Ok(FileContentMCP { path: params.path, text_content: None, image_data_base64: Some(BASE64_STANDARD.encode(&bytes)), mime_type, lines_read: None, total_lines: None, truncated: None, error: None })
    } else {
        let full_content = deps.app_handle.fs().read_text_file(&path).await.map_err(|e| AppError::PluginError{plugin:"fs".into(), message:e.to_string()})?;
        let mut lines_iter = full_content.lines();
        let mut content_vec = Vec::new();
        let mut current_line_idx = 0;
        let mut total_lines_count = 0;
        let read_limit = params.length.unwrap_or(config_guard.file_read_line_limit);
        // No need to drop config_guard explicitly here, it's fine.

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

pub async fn mcp_write_file(deps: &ToolDependencies, params: WriteFileParamsMCP) -> Result<FileOperationResultMCP, AppError> {
    let config_guard = deps.config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock: {}", e)))?;
    let path = validate_and_normalize_path(params.path, &config_guard, false, true)?;
    let lines: Vec<&str> = params.content.lines().collect();
    if lines.len() > config_guard.file_write_line_limit { return Err(AppError::EditError(format!("Content exceeds line limit {}. Received {}.", config_guard.file_write_line_limit, lines.len()))); }
    
    let final_content = if params.mode == WriteModeMCP::Append && deps.app_handle.fs().exists(&path).await.unwrap_or(false) {
        let existing = deps.app_handle.fs().read_text_file(&path).await.unwrap_or_default();
        normalize_line_endings(params.content, detect_line_ending(&existing))
    } else { normalize_line_endings(params.content, if cfg!(windows) {LineEndingStyle::CrLf} else {LineEndingStyle::Lf}) };
    
    if !deps.app_handle.fs_scope().is_allowed(&path) { return Err(AppError::PathNotAllowed(format!("FS scope disallows write: {}", path.display()))); }
    
    match params.mode {
        WriteModeMCP::Rewrite => { deps.app_handle.fs().write_text_file(&path, &final_content).await.map_err(|e|AppError::PluginError{plugin:"fs".into(), message:e.to_string()})?; }
        WriteModeMCP::Append => {
            let mut current_content = String::new();
            if deps.app_handle.fs().exists(&path).await.unwrap_or(false) { current_content = deps.app_handle.fs().read_text_file(&path).await.unwrap_or_default(); }
            if !current_content.is_empty() && !current_content.ends_with(['\n', '\r']) { 
                let detected_ending = detect_line_ending(current_content); // Pass &str
                current_content.push_str(detected_ending.as_str()); 
            }
            current_content.push_str(&final_content);
            deps.app_handle.fs().write_text_file(&path, current_content).await.map_err(|e|AppError::PluginError{plugin:"fs".into(), message:e.to_string()})?;
        }
    }
    Ok(FileOperationResultMCP { success: true, path: params.path, message: format!("Successfully {} content.", if params.mode == WriteModeMCP::Append {"appended"} else {"wrote"})})
}

pub async fn mcp_create_directory(deps: &ToolDependencies, params: CreateDirectoryParamsMCP) -> Result<FileOperationResultMCP, AppError> {
    let config_guard = deps.config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock: {}", e)))?;
    let path = validate_and_normalize_path(params.path, &config_guard, false, true)?;
    if !deps.app_handle.fs_scope().is_allowed(&path) { return Err(AppError::PathNotAllowed(format!("FS scope disallows dir creation: {}", path.display()))); }
    deps.app_handle.fs().create_dir(&path, true).await.map_err(|e|AppError::PluginError{plugin:"fs".into(), message:e.to_string()})?;
    Ok(FileOperationResultMCP { success: true, path: params.path, message: "Directory created.".to_string() })
}

pub async fn mcp_list_directory(deps: &ToolDependencies, params: ListDirectoryParamsMCP) -> Result<ListDirectoryResultMCP, AppError> {
    let config_guard = deps.config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock: {}", e)))?;
    let path = validate_and_normalize_path(params.path, &config_guard, true, false)?;
    if !deps.app_handle.fs_scope().is_allowed(&path) { return Err(AppError::PathNotAllowed(format!("FS scope disallows list: {}", path.display()))); }
    let entries_data = deps.app_handle.fs().read_dir(&path, false).await.map_err(|e|AppError::PluginError{plugin:"fs".into(), message:e.to_string()})?;
    let mut entries = entries_data.into_iter().map(|e| format!("{} {}", if e.is_dir {"[DIR]"} else {"[FILE]"}, e.name.unwrap_or_default())).collect::<Vec<_>>();
    entries.sort();
    Ok(ListDirectoryResultMCP { path: params.path, entries })
}

pub async fn mcp_move_file(deps: &ToolDependencies, params: MoveFileParamsMCP) -> Result<FileOperationResultMCP, AppError> {
    let config_guard = deps.config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock: {}", e)))?;
    let source_path = validate_and_normalize_path(params.source, &config_guard, true, false)?;
    let dest_path = validate_and_normalize_path(params.destination, &config_guard, false, true)?;
    if !deps.app_handle.fs_scope().is_allowed(&source_path) || !deps.app_handle.fs_scope().is_allowed(&dest_path.parent().unwrap_or(&dest_path)) {
        return Err(AppError::PathNotAllowed(format!("FS scope disallows move from {} or to {}", source_path.display(), dest_path.parent().unwrap_or(&dest_path).display())));
    }
    deps.app_handle.fs().rename_file(&source_path, &dest_path).await.map_err(|e|AppError::PluginError{plugin:"fs".into(), message:e.to_string()})?;
    Ok(FileOperationResultMCP { success: true, path: params.destination, message: format!("Moved {} to {}.", params.source, params.destination) })
}

pub async fn mcp_get_file_info(deps: &ToolDependencies, params: GetFileInfoParamsMCP) -> Result<FileInfoResultMCP, AppError> {
    let config_guard = deps.config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock: {}", e)))?;
    let path = validate_and_normalize_path(params.path, &config_guard, true, false)?;
    if !deps.app_handle.fs_scope().is_allowed(&path) { return Err(AppError::PathNotAllowed(format!("FS scope disallows info: {}", path.display()))); }
    let metadata = deps.app_handle.fs().metadata(&path).await.map_err(|e|AppError::PluginError{plugin:"fs".into(), message:e.to_string()})?;
    let to_iso = |st: Option<std::time::SystemTime>| st.map(chrono::DateTime::<chrono::Utc>::from).map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
    let std_meta = std::fs::metadata(&path)?; 
    let perms = {
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            Some(format!("{:03o}", std_meta.permissions().mode() & 0o777))
        }
        #[cfg(not(unix))] {
            None::<String>
        }
    };
    Ok(FileInfoResultMCP { path: params.path, size: metadata.len(), is_dir: metadata.is_dir(), is_file: metadata.is_file(), modified_iso: to_iso(metadata.modified), created_iso: to_iso(metadata.created), accessed_iso: to_iso(metadata.accessed), permissions_octal: perms })
}

pub async fn mcp_read_multiple_files(deps: &ToolDependencies, params: ReadMultipleFilesParamsMCP) -> Result<ReadMultipleFilesResultMCP, AppError> {
    let config_guard = deps.config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock: {}", e)))?;
    let mut results = Vec::new();
    let http_client = reqwest::Client::new();
    for path_str in params.paths {
        let is_url = path_str.starts_with("http://") || path_str.starts_with("https://");
        // Re-implement single read logic here to avoid calling another MCP tool which might cause issues or be less efficient
        let content_res = if is_url {
            read_file_from_url_mcp_internal(&http_client, &path_str, &deps.app_handle).await
        } else {
            match validate_and_normalize_path(&path_str, &config_guard, true, false) {
                Ok(val_path) => {
                    if !deps.app_handle.fs_scope().is_allowed(&val_path) { Err(AppError::PathNotAllowed(format!("FS scope disallows read: {}", val_path.display()))) }
                    else {
                        let mime = mime_guess::from_path(&val_path).first_or_octet_stream().to_string();
                        if is_image_mime_mcp(&mime) {
                            deps.app_handle.fs().read_binary(&val_path).await.map_err(|e|AppError::PluginError{plugin:"fs".into(), message:e.to_string()})
                                .map(|b| FileContentMCP{path:path_str.clone(), text_content:None, image_data_base64:Some(BASE64_STANDARD.encode(&b)), mime_type:mime, lines_read:None, total_lines:None, truncated:None, error:None})
                        } else {
                            deps.app_handle.fs().read_text_file(&val_path).await.map_err(|e|AppError::PluginError{plugin:"fs".into(), message:e.to_string()})
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


async fn search_files_recursive_mcp(app_handle: &tauri::AppHandle, dir: PathBuf, pattern: &str, matches: &mut Vec<String>, depth: usize, max_depth: usize, root: &Path, cfg: &RwLockReadGuard<'_, Config>) -> Result<(), AppError> {
    if depth > max_depth { return Ok(()); }
    if !app_handle.fs_scope().is_allowed(&dir) || validate_and_normalize_path(dir.to_str().unwrap_or_default(), cfg, true, false).is_err() { return Ok(()); }
    
    let entries_result = app_handle.fs().read_dir(&dir, false).await;
    let entries = match entries_result {
        Ok(e) => e,
        Err(tauri_fs_err) => {
            warn!(path = %dir.display(), error = %tauri_fs_err, "Could not read directory during search_files_recursive_mcp");
            return Ok(()); // Skip unreadable directories
        }
    };

    for entry_data in entries {
        let name_lower = entry_data.name.as_ref().map_or_else(String::new, |n| n.to_lowercase());
        if name_lower.contains(pattern) {
            if let Ok(rel_path) = entry_data.path.strip_prefix(root) { matches.push(rel_path.to_string_lossy().into_owned()); }
            else { matches.push(entry_data.path.to_string_lossy().into_owned()); }
        }
        if entry_data.is_dir && depth < max_depth { search_files_recursive_mcp(app_handle, entry_data.path, pattern, matches, depth + 1, max_depth, root, cfg).await?; }
    }
    Ok(())
}

pub async fn mcp_search_files(deps: &ToolDependencies, params: SearchFilesParamsMCP) -> Result<SearchFilesResultMCP, AppError> {
    let config_guard = deps.config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock: {}", e)))?;
    let root_search = validate_and_normalize_path(params.path, &config_guard, true, false)?;
    let files_root_clone = config_guard.files_root.clone();
    
    // Clone necessary parts of deps and config_guard for the async block
    let app_handle_clone = deps.app_handle.clone();
    let config_guard_clone = deps.config_state.clone(); // Clone Arc for async block

    let search_op = async move {
        let mut matches = Vec::new();
        let pattern_lower = params.pattern.to_lowercase();
        // Re-acquire read lock inside async block
        let current_config_guard = config_guard_clone.read().map_err(|_| AppError::ConfigError(anyhow::anyhow!("Config lock error in search task")))?;

        if params.recursive { 
            search_files_recursive_mcp(&app_handle_clone, root_search.clone(), &pattern_lower, &mut matches, 0, params.max_depth, &files_root_clone, current_config_guard).await?; 
        } else {
            if !app_handle_clone.fs_scope().is_allowed(&root_search) || validate_and_normalize_path(root_search.to_str().unwrap_or_default(), current_config_guard, true, false).is_err() { 
                 warn!(path = %root_search.display(), "Search skipped: path not allowed by scope or config.");
                 return Ok(matches);
            }
            let dir_entries = app_handle_clone.fs().read_dir(&root_search, false).await.map_err(|e|AppError::PluginError{plugin:"fs".into(), message:e.to_string()})?;
            for entry in dir_entries {
                if entry.name.as_ref().map_or(false, |n|n.to_lowercase().contains(&pattern_lower)) {
                    if let Ok(rel) = entry.path.strip_prefix(&files_root_clone) { matches.push(rel.to_string_lossy().into_owned()); } else { matches.push(entry.path.to_string_lossy().into_owned()); }
                }
            }
        }
        matches.sort();
        Ok(matches)
    };
    
    drop(config_guard); // Drop original guard before await

    match timeout(Duration::from_millis(params.timeout_ms.unwrap_or(FILE_SEARCH_TIMEOUT_MS_MCP)), search_op).await {
        Ok(Ok(m)) => Ok(SearchFilesResultMCP { path: params.path, pattern: params.pattern, matches: m, timed_out: false }),
        Ok(Err(e)) => Err(e),
        Err(_) => Ok(SearchFilesResultMCP { path: params.path, pattern: params.pattern, matches: vec![], timed_out: true }),
    }
}