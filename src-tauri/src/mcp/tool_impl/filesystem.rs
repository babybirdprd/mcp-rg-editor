use crate::config::Config;
use crate::error::AppError;
use crate::mcp::handler::ToolDependencies;
use crate::utils::path_utils::validate_and_normalize_path;
use crate::utils::line_ending_handler::{detect_line_ending, normalize_line_endings, LineEndingStyle};

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock as StdRwLock}; // MODIFIED: Added Arc, RwLock
use tauri_plugin_fs::FsExt;
use tokio::fs as tokio_fs; 
use tokio::io::AsyncWriteExt; 

use tracing::{debug, warn, instrument};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use tokio::time::{timeout, Duration};
use chrono::{DateTime, Utc};


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
pub struct DirEntryMCP {
    pub path: String,
    pub name: Option<String>,
    pub is_dir: bool,
}
#[derive(Debug, Serialize)]
pub struct ListDirectoryResultMCP { pub path: String, pub entries: Vec<DirEntryMCP> }

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

#[instrument(skip(http_client), fields(url = %url_str))]
async fn read_file_from_url_mcp_internal(
    http_client: &reqwest::Client,
    url_str: &str,
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
    if params.is_url {
        let client = reqwest::Client::new();
        // No config_guard needed for URL fetching, so it's not held across await.
        return read_file_from_url_mcp_internal(&client, &params.path).await;
    }

    let (path, read_limit) = { // Scope for config_guard
        let config_guard = deps.config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock for read_file: {}", e)))?;
        let p = validate_and_normalize_path(&params.path, &*config_guard, true, false)?;
        let limit = params.length.unwrap_or(config_guard.file_read_line_limit);
        (p, limit)
    }; // config_guard is dropped here

    if !deps.app_handle.fs_scope().is_allowed(&path) { return Err(AppError::PathNotAllowed(format!("FS scope disallows read: {}", path.display()))); }

    let mime_type = mime_guess::from_path(&path).first_or_octet_stream().to_string();
    if is_image_mime_mcp(&mime_type) {
        let bytes = tokio_fs::read(&path).await.map_err(|e| AppError::TokioIoError(e.to_string()))?;
        Ok(FileContentMCP { path: params.path, text_content: None, image_data_base64: Some(BASE64_STANDARD.encode(&bytes)), mime_type, lines_read: None, total_lines: None, truncated: None, error: None })
    } else {
        let full_content = tokio_fs::read_to_string(&path).await.map_err(|e| AppError::TokioIoError(e.to_string()))?;
        let lines_iter = full_content.lines();
        let mut content_vec = Vec::new();
        let mut current_line_idx = 0;
        let mut total_lines_count = 0;
        
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
    let (path, write_line_limit) = { // Scope for config_guard
        let config_guard = deps.config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock for write_file: {}", e)))?;
        let p = validate_and_normalize_path(&params.path, &*config_guard, false, true)?;
        let limit = config_guard.file_write_line_limit;
        (p, limit)
    }; // config_guard is dropped here

    let lines: Vec<&str> = params.content.lines().collect();
    if lines.len() > write_line_limit { return Err(AppError::EditError(format!("Content exceeds line limit {}. Received {}.", write_line_limit, lines.len()))); }

    let final_content_str = if params.mode == WriteModeMCP::Append && tokio_fs::try_exists(&path).await.unwrap_or(false) {
        let existing_content_str = tokio_fs::read_to_string(&path).await.unwrap_or_default();
        normalize_line_endings(&params.content, detect_line_ending(&existing_content_str))
    } else { normalize_line_endings(&params.content, if cfg!(windows) {LineEndingStyle::CrLf} else {LineEndingStyle::Lf}) };

    if !deps.app_handle.fs_scope().is_allowed(&path) { return Err(AppError::PathNotAllowed(format!("FS scope disallows write: {}", path.display()))); }

    if params.mode == WriteModeMCP::Append {
        let mut file = tokio_fs::OpenOptions::new().append(true).create(true).open(&path).await.map_err(|e| AppError::TokioIoError(e.to_string()))?;
        file.write_all(final_content_str.as_bytes()).await.map_err(|e| AppError::TokioIoError(e.to_string()))?;
    } else {
        tokio_fs::write(&path, final_content_str).await.map_err(|e| AppError::TokioIoError(e.to_string()))?;
    }

    Ok(FileOperationResultMCP { success: true, path: params.path, message: format!("Successfully {} content.", if params.mode == WriteModeMCP::Append {"appended"} else {"wrote"})})
}

#[instrument(skip(deps, params), fields(path = %params.path))]
pub async fn mcp_create_directory(deps: &ToolDependencies, params: CreateDirectoryParamsMCP) -> Result<FileOperationResultMCP, AppError> {
    let path = { // Scope for config_guard
        let config_guard = deps.config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock for create_directory: {}", e)))?;
        validate_and_normalize_path(&params.path, &*config_guard, false, true)?
    }; // config_guard is dropped here
    if !deps.app_handle.fs_scope().is_allowed(&path) { return Err(AppError::PathNotAllowed(format!("FS scope disallows dir creation: {}", path.display()))); }
    tokio_fs::create_dir_all(&path).await.map_err(|e|AppError::TokioIoError(e.to_string()))?;
    Ok(FileOperationResultMCP { success: true, path: params.path, message: "Directory created.".to_string() })
}

#[instrument(skip(deps, params), fields(path = %params.path))]
pub async fn mcp_list_directory(deps: &ToolDependencies, params: ListDirectoryParamsMCP) -> Result<ListDirectoryResultMCP, AppError> {
    let path = { // Scope for config_guard
        let config_guard = deps.config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock for list_directory: {}", e)))?;
        validate_and_normalize_path(&params.path, &*config_guard, true, false)?
    }; // config_guard is dropped here
    if !deps.app_handle.fs_scope().is_allowed(&path) { return Err(AppError::PathNotAllowed(format!("FS scope disallows list: {}", path.display()))); }
    
    let mut entries_mcp = Vec::new();
    let mut read_dir = tokio_fs::read_dir(&path).await.map_err(|e| AppError::TokioIoError(e.to_string()))?;
    while let Some(entry_res) = read_dir.next_entry().await.map_err(|e| AppError::TokioIoError(e.to_string()))? {
        let entry = entry_res;
        let entry_path = entry.path();
        let file_type = entry.file_type().await.map_err(|e| AppError::TokioIoError(e.to_string()))?;
        entries_mcp.push(DirEntryMCP {
            path: entry_path.to_string_lossy().into_owned(),
            name: entry.file_name().into_string().ok(),
            is_dir: file_type.is_dir(),
        });
    }
    Ok(ListDirectoryResultMCP { path: params.path, entries: entries_mcp })
}

#[instrument(skip(deps, params), fields(source = %params.source, dest = %params.destination))]
pub async fn mcp_move_file(deps: &ToolDependencies, params: MoveFileParamsMCP) -> Result<FileOperationResultMCP, AppError> {
    let (source_path, dest_path) = { // Scope for config_guard
        let config_guard = deps.config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock for move_file: {}", e)))?;
        let s_path = validate_and_normalize_path(&params.source, &*config_guard, true, false)?;
        let d_path = validate_and_normalize_path(&params.destination, &*config_guard, false, true)?;
        (s_path, d_path)
    }; // config_guard is dropped here
    if !deps.app_handle.fs_scope().is_allowed(&source_path) || !deps.app_handle.fs_scope().is_allowed(&dest_path.parent().unwrap_or(&dest_path)) {
        return Err(AppError::PathNotAllowed(format!("FS scope disallows move from {} or to {}", source_path.display(), dest_path.parent().unwrap_or(&dest_path).display())));
    }
    tokio_fs::rename(&source_path, &dest_path).await.map_err(|e|AppError::TokioIoError(e.to_string()))?;
    Ok(FileOperationResultMCP { success: true, path: params.destination.clone(), message: format!("Moved {} to {}.", params.source, params.destination) })
}

#[instrument(skip(deps, params), fields(path = %params.path))]
pub async fn mcp_get_file_info(deps: &ToolDependencies, params: GetFileInfoParamsMCP) -> Result<FileInfoResultMCP, AppError> {
    let path = { // Scope for config_guard
        let config_guard = deps.config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock for get_file_info: {}", e)))?;
        validate_and_normalize_path(&params.path, &*config_guard, true, false)?
    }; // config_guard is dropped here
    if !deps.app_handle.fs_scope().is_allowed(&path) { return Err(AppError::PathNotAllowed(format!("FS scope disallows info: {}", path.display()))); }

    let std_meta = tokio_fs::metadata(&path).await.map_err(|e| AppError::TokioIoError(e.to_string()))?;

    let to_iso_from_system_time = |st_res: Result<std::time::SystemTime, std::io::Error>| {
        st_res.ok().map(|st| {
            let dt: DateTime<Utc> = st.into();
            dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
        })
    };

    let perms = {
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            Some(format!("{:03o}", std_meta.permissions().mode() & 0o777))
        }
        #[cfg(not(unix))] { None::<String> }
    };
    Ok(FileInfoResultMCP {
        path: params.path,
        size: std_meta.len(),
        is_dir: std_meta.is_dir(),
        is_file: std_meta.is_file(),
        modified_iso: to_iso_from_system_time(std_meta.modified()),
        created_iso: to_iso_from_system_time(std_meta.created()),
        accessed_iso: to_iso_from_system_time(std_meta.accessed()),
        permissions_octal: perms
    })
}

#[instrument(skip(deps, params), fields(paths_count = %params.paths.len()))]
pub async fn mcp_read_multiple_files(deps: &ToolDependencies, params: ReadMultipleFilesParamsMCP) -> Result<ReadMultipleFilesResultMCP, AppError> {
    let mut results = Vec::new();
    let http_client = reqwest::Client::new();

    for path_str_from_params in params.paths {
        let path_str = path_str_from_params.clone();
        let is_url = path_str.starts_with("http://") || path_str.starts_with("https://");

        let content_res = if is_url {
             // No config_guard needed for URL fetching
            read_file_from_url_mcp_internal(&http_client, &path_str).await
        } else {
            let validated_path_res = { // Scope for config_guard
                let config_guard = deps.config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock for multi-read path validation: {}", e)))?;
                validate_and_normalize_path(&path_str, &*config_guard, true, false)
            }; // config_guard dropped

            match validated_path_res {
                Ok(val_path) => {
                    if !deps.app_handle.fs_scope().is_allowed(&val_path) { Err(AppError::PathNotAllowed(format!("FS scope disallows read: {}", val_path.display()))) }
                    else {
                        let mime = mime_guess::from_path(&val_path).first_or_octet_stream().to_string();
                        if is_image_mime_mcp(&mime) {
                            tokio_fs::read(&val_path).await
                                .map_err(|e|AppError::TokioIoError(e.to_string()))
                                .map(|b| FileContentMCP{path:path_str.clone(), text_content:None, image_data_base64:Some(BASE64_STANDARD.encode(&b)), mime_type:mime, lines_read:None, total_lines:None, truncated:None, error:None})
                        } else {
                            tokio_fs::read_to_string(&val_path).await
                                .map_err(|e|AppError::TokioIoError(e.to_string()))
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

#[instrument(skip(app_handle, pattern_lower, matches, config_state), fields(dir = %dir_to_search.display()))]
async fn search_files_recursive_mcp_internal(
    app_handle: &tauri::AppHandle,
    dir_to_search: PathBuf,
    pattern_lower: &str,
    matches: &mut Vec<String>,
    current_depth: usize,
    max_depth: usize,
    files_root_for_relative_path: &Path,
    config_state: &Arc<StdRwLock<Config>>, // MODIFIED: Accept Arc<RwLock<Config>>
) -> Result<(), AppError> {
    if current_depth > max_depth { return Ok(()); }

    if !app_handle.fs_scope().is_allowed(&dir_to_search) {
        warn!(path = %dir_to_search.display(), "Search skipped: path not allowed by FS scope.");
        return Ok(());
    }
    { // Scope for config_guard
        let config_guard = config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock for recursive search validation: {}", e)))?;
        if validate_and_normalize_path(dir_to_search.to_str().unwrap_or_default(), &*config_guard, true, false).is_err() {
            warn!(path = %dir_to_search.display(), "Search skipped: path not allowed by config.");
            return Ok(());
        }
    } // config_guard dropped

    let mut read_dir = match tokio_fs::read_dir(&dir_to_search).await {
        Ok(rd) => rd,
        Err(e) => {
            warn!(path = %dir_to_search.display(), error = %e, "Could not read directory during search_files");
            return Ok(()); 
        }
    };
    
    while let Some(entry_res) = read_dir.next_entry().await.map_err(|e| AppError::TokioIoError(e.to_string()))? {
        let entry = entry_res;
        let entry_name_os = entry.file_name();
        let entry_name_lower = entry_name_os.to_string_lossy().to_lowercase();
        let full_path = entry.path();

        if entry_name_lower.contains(pattern_lower) {
            if let Ok(relative_path) = full_path.strip_prefix(files_root_for_relative_path) {
                 matches.push(relative_path.to_string_lossy().into_owned());
            } else {
                matches.push(full_path.to_string_lossy().into_owned());
            }
        }
        if entry.file_type().await.map_err(|e| AppError::TokioIoError(e.to_string()))?.is_dir() && current_depth < max_depth {
            Box::pin(search_files_recursive_mcp_internal(app_handle, full_path, pattern_lower, matches, current_depth + 1, max_depth, files_root_for_relative_path, config_state)).await?;
        }
    }
    Ok(())
}

#[instrument(skip(deps, params), fields(path = %params.path, pattern = %params.pattern))]
pub async fn mcp_search_files(deps: &ToolDependencies, params: SearchFilesParamsMCP) -> Result<SearchFilesResultMCP, AppError> {
    let (root_search_path, files_root_clone) = { // Scope for config_guard
        let config_guard = deps.config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock for search_files: {}", e)))?;
        let rsp = validate_and_normalize_path(&params.path, &*config_guard, true, false)?;
        let frc = config_guard.files_root.clone();
        (rsp, frc)
    }; // config_guard dropped

    let app_handle_clone = deps.app_handle.clone();
    let pattern_lower_clone = params.pattern.to_lowercase();
    let max_depth_clone = params.max_depth;
    let recursive_clone = params.recursive;
    let config_state_clone = deps.config_state.clone(); // Clone Arc for passing to recursive


    let search_operation = async {
        let mut matches = Vec::new();

        if recursive_clone {
            Box::pin(search_files_recursive_mcp_internal(&app_handle_clone, root_search_path.clone(), &pattern_lower_clone, &mut matches, 0, max_depth_clone, &files_root_clone, &config_state_clone)).await?;
        } else {
            if !app_handle_clone.fs_scope().is_allowed(&root_search_path) {
                 let temp_config_guard_for_validation = config_state_clone.read().map_err(|e| AppError::ConfigError(format!("Config lock for non-recursive validation: {}", e)))?;
                 if validate_and_normalize_path(root_search_path.to_str().unwrap_or_default(), &*temp_config_guard_for_validation, true, false).is_err() {
                    warn!(path = %root_search_path.display(), "Search skipped: path not allowed by scope or config.");
                    return Ok(matches);
                 }
            }
            let mut read_dir = tokio_fs::read_dir(&root_search_path).await.map_err(|e| AppError::TokioIoError(e.to_string()))?;
            while let Some(entry_res) = read_dir.next_entry().await.map_err(|e| AppError::TokioIoError(e.to_string()))? {
                let entry = entry_res;
                let entry_name_os = entry.file_name();
                let entry_name_lower = entry_name_os.to_string_lossy().to_lowercase();
                 if entry_name_lower.contains(&pattern_lower_clone) {
                    if let Ok(relative_path) = entry.path().strip_prefix(&files_root_clone) {
                         matches.push(relative_path.to_string_lossy().into_owned());
                    } else { matches.push(entry.path().to_string_lossy().into_owned()); }
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