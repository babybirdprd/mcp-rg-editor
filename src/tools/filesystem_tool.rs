use crate::config::Config;
use crate::error::AppError;
use crate::utils::path_utils::{validate_path_access, validate_parent_path_access};
use crate::utils::line_ending_handler::{detect_line_ending, normalize_line_endings, LineEndingStyle};
use serde::{Deserialize, Serialize};
// Removed std::path::PathBuf as it's not directly used here for struct fields
use std::sync::Arc; // Keep Arc
use std::sync::RwLock as StdRwLock; // For Config
use tokio::fs;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader}; // Removed BufWriter as not used
use tokio::time::timeout;
use tracing::{debug, instrument, warn};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};


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
pub struct ReadMultipleFilesParams {
    pub paths: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct WriteFileParams {
    pub path: String,
    pub content: String,
    #[serde(default = "default_rewrite_mode")]
    pub mode: WriteMode,
}
fn default_rewrite_mode() -> WriteMode { WriteMode::Rewrite }

#[derive(Debug, Deserialize, PartialEq, Clone, Copy)]
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
pub struct SearchFilesParams {
    pub path: String,
    pub pattern: String,
    #[serde(rename = "timeoutMs")]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct GetFileInfoParams {
    pub path: String,
}

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
pub struct SearchFilesResult {
    pub path: String,
    pub pattern: String,
    pub matches: Vec<String>,
    pub timed_out: bool,
}

#[derive(Debug, Serialize)]
pub struct FileInfoResult {
    pub path: String,
    pub size: u64,
    pub is_dir: bool,
    pub is_file: bool,
    pub modified_iso: Option<String>,
    pub created_iso: Option<String>,
    pub accessed_iso: Option<String>,
    pub permissions_octal: Option<String>,
}

#[derive(Debug)] // Added Debug
pub struct FilesystemManager {
    config: Arc<StdRwLock<Config>>, // Changed to StdRwLock
    http_client: reqwest::Client,
}

const URL_FETCH_TIMEOUT_MS: u64 = 30000;
const FILE_OPERATION_TIMEOUT_MS: u64 = 30000;

fn is_image_mime(mime_type: &str) -> bool {
    mime_type.starts_with("image/") && 
    (mime_type.ends_with("/png") || mime_type.ends_with("/jpeg") || mime_type.ends_with("/gif") || mime_type.ends_with("/webp"))
}

impl FilesystemManager {
    pub fn new(config: Arc<StdRwLock<Config>>) -> Self { // Changed to StdRwLock
        Self {
            config,
            http_client: reqwest::Client::new(),
        }
    }

    async fn read_file_from_url(&self, url_str: &str) -> Result<FileContent, AppError> {
        debug!(url = %url_str, "Reading file from URL");
        let request = self.http_client.get(url_str).build()?;
        
        let response = match timeout(Duration::from_millis(URL_FETCH_TIMEOUT_MS), self.http_client.execute(request)).await {
            Ok(Ok(resp)) => resp,
            Ok(Err(e)) => return Err(AppError::ReqwestError(e)),
            Err(_) => return Err(AppError::TimeoutError(format!("URL fetch timed out after {}ms: {}", URL_FETCH_TIMEOUT_MS, url_str))),
        };

        if !response.status().is_success() {
            return Err(AppError::ReqwestError(response.error_for_status().unwrap_err()));
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
            let bytes = response.bytes().await?;
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
            let text_content = response.text().await?;
            let lines: Vec<&str> = text_content.lines().collect();
            Ok(FileContent {
                path: url_str.to_string(),
                text_content: Some(text_content),
                image_data_base64: None,
                mime_type,
                lines_read: Some(lines.len()),
                total_lines: Some(lines.len()),
                truncated: Some(false),
                error: None,
            })
        }
    }

    #[instrument(skip(self, params), fields(path = %params.path, is_url = %params.is_url))]
    pub async fn read_file(&self, params: &ReadFileParams) -> Result<FileContent, AppError> {
        if params.is_url {
            return self.read_file_from_url(&params.path).await; // Corrected: params.path
        }
        let config_guard = self.config.read().map_err(|_| AppError::ConfigError(anyhow::anyhow!("Config lock poisoned")))?;
        let path = validate_path_access(&params.path, &config_guard, true)?; // Corrected: params.path
        debug!(local_path = %path.display(), "Reading file from disk");

        let mime_type = mime_guess::from_path(&path).first_or_octet_stream().to_string();

        if is_image_mime(&mime_type) {
            let bytes = fs::read(&path).await?;
            let base64_data = BASE64_STANDARD.encode(&bytes);
            Ok(FileContent {
                path: params.path.clone(),
                text_content: None,
                image_data_base64: Some(base64_data),
                mime_type,
                lines_read: None,
                total_lines: None,
                truncated: None,
                error: None,
            })
        } else {
            let file = fs::File::open(&path).await?;
            let reader = BufReader::new(file);
            let mut lines_iter = reader.lines();

            let mut content_vec = Vec::new();
            let mut current_line_idx = 0;
            let mut total_lines_count = 0;
            let read_limit = params.length.unwrap_or(config_guard.file_read_line_limit);
            drop(config_guard); // Release lock
            
            while let Some(line_res) = lines_iter.next_line().await.map_err(AppError::from)? {
                total_lines_count += 1;
                if current_line_idx >= params.offset && content_vec.len() < read_limit {
                    content_vec.push(line_res);
                }
                current_line_idx += 1;
                 // Check if we've hit the read limit AND there are more lines potentially available
                if content_vec.len() >= read_limit && (params.offset + content_vec.len()) < total_lines_count {
                    break; 
                }
            }
            
            let lines_read_count = content_vec.len();
            let text_content = content_vec.join("\n");
            // Truncation occurs if we started reading from an offset, or if we read up to the limit and there were more lines than what we read plus the offset.
            let is_truncated = params.offset > 0 || (lines_read_count == read_limit && (params.offset + lines_read_count) < total_lines_count);


            Ok(FileContent {
                path: params.path.clone(),
                text_content: Some(text_content),
                image_data_base64: None,
                mime_type,
                lines_read: Some(lines_read_count),
                total_lines: Some(total_lines_count),
                truncated: Some(is_truncated),
                error: None,
            })
        }
    }

    #[instrument(skip(self, params))]
    pub async fn read_multiple_files(&self, params: &ReadMultipleFilesParams) -> Result<Vec<FileContent>, AppError> {
        let mut results = Vec::new();
        let config_guard = self.config.read().map_err(|_| AppError::ConfigError(anyhow::anyhow!("Config lock poisoned")))?;
        let default_read_limit = config_guard.file_read_line_limit;
        drop(config_guard); // Release lock early

        for path_str in &params.paths { // Corrected: &params.paths
            let read_params = ReadFileParams {
                path: path_str.clone(),
                is_url: false, 
                offset: 0,
                length: Some(default_read_limit),
            };
            match self.read_file(&read_params).await {
                Ok(content) => results.push(content),
                Err(e) => {
                    warn!(path = %path_str, error = %e, "Failed to read one of multiple files");
                    results.push(FileContent {
                        path: path_str.clone(),
                        text_content: None,
                        image_data_base64: None,
                        mime_type: "error/unknown".to_string(),
                        lines_read: None,
                        total_lines: None,
                        truncated: None,
                        error: Some(e.to_string()),
                    });
                }
            }
        }
        Ok(results)
    }


    #[instrument(skip(self, params), fields(path = %params.path, mode = ?params.mode))]
    pub async fn write_file(&self, params: &WriteFileParams) -> Result<FileOperationResult, AppError> {
        let config_guard = self.config.read().map_err(|_| AppError::ConfigError(anyhow::anyhow!("Config lock poisoned")))?;
        let path = validate_parent_path_access(&params.path, &config_guard)?; // Corrected: params.path
        debug!(write_path = %path.display(), mode = ?params.mode, "Writing file");

        let lines: Vec<&str> = params.content.lines().collect();
        if lines.len() > config_guard.file_write_line_limit {
            return Err(AppError::EditError(format!(
                "Content exceeds line limit of {}. Received {} lines. Please send content in smaller chunks.",
                config_guard.file_write_line_limit,
                lines.len()
            )));
        }
        
        let final_content = if params.mode == WriteMode::Append && path.exists() {
            let existing_content_str = fs::read_to_string(&path).await.unwrap_or_default();
            let detected_ending = detect_line_ending(&existing_content_str);
            normalize_line_endings(&params.content, detected_ending) // Corrected: params.content
        } else {
            let system_ending = if cfg!(windows) { LineEndingStyle::CrLf } else { LineEndingStyle::Lf };
            normalize_line_endings(&params.content, system_ending) // Corrected: params.content
        };
        drop(config_guard); // Release lock


        let mut file = match params.mode {
            WriteMode::Rewrite => fs::File::create(&path).await?,
            WriteMode::Append => fs::OpenOptions::new().append(true).create(true).open(&path).await?,
        };
        file.write_all(final_content.as_bytes()).await?;
        file.flush().await?;

        Ok(FileOperationResult {
            success: true,
            path: params.path.clone(),
            message: format!("Successfully {} {} lines to file.", if params.mode == WriteMode::Append {"appended"} else {"wrote"}, lines.len()),
        })
    }
    
    #[instrument(skip(self, params), fields(path = %params.path))]
    pub async fn create_directory(&self, params: &CreateDirectoryParams) -> Result<FileOperationResult, AppError> {
        let config_guard = self.config.read().map_err(|_| AppError::ConfigError(anyhow::anyhow!("Config lock poisoned")))?;
        let path = validate_parent_path_access(&params.path, &config_guard)?; // Corrected: params.path
        drop(config_guard);
        debug!(create_dir_path = %path.display(), "Creating directory");
        fs::create_dir_all(&path).await?;
        Ok(FileOperationResult {
            success: true,
            path: params.path.clone(),
            message: "Directory created successfully.".to_string(),
        })
    }

    #[instrument(skip(self, params), fields(path = %params.path))]
    pub async fn list_directory(&self, params: &ListDirectoryParams) -> Result<ListDirectoryResult, AppError> {
        let config_guard = self.config.read().map_err(|_| AppError::ConfigError(anyhow::anyhow!("Config lock poisoned")))?;
        let path = validate_path_access(&params.path, &config_guard, true)?; // Corrected: params.path
        drop(config_guard);
        debug!(list_dir_path = %path.display(), "Listing directory");
        let mut entries = Vec::new();
        let mut read_dir = fs::read_dir(&path).await?;
        while let Some(entry_res) = read_dir.next_entry().await? {
            let entry_path = entry_res.path();
            let entry_name = entry_res.file_name().to_string_lossy().to_string();
            let prefix = if entry_path.is_dir() { "[DIR] " } else { "[FILE]" }; // Corrected: [DIR] space
            entries.push(format!("{} {}", prefix, entry_name));
        }
        entries.sort();
        Ok(ListDirectoryResult {
            path: params.path.clone(),
            entries,
        })
    }

    #[instrument(skip(self, params), fields(source = %params.source, dest = %params.destination))]
    pub async fn move_file(&self, params: &MoveFileParams) -> Result<FileOperationResult, AppError> {
        let config_guard = self.config.read().map_err(|_| AppError::ConfigError(anyhow::anyhow!("Config lock poisoned")))?;
        let source_path = validate_path_access(&params.source, &config_guard, true)?; // Corrected: params.source
        let dest_path = validate_parent_path_access(&params.destination, &config_guard)?;  // Corrected: params.destination
        drop(config_guard);
        debug!(move_source = %source_path.display(), move_dest = %dest_path.display(), "Moving file/directory");
        fs::rename(&source_path, &dest_path).await?;
        Ok(FileOperationResult {
            success: true,
            path: params.destination.clone(), 
            message: format!("Successfully moved {} to {}.", params.source, params.destination),
        })
    }

    #[instrument(skip(self, params), fields(path = %params.path, pattern = %params.pattern))]
    pub async fn search_files(&self, params: &SearchFilesParams) -> Result<SearchFilesResult, AppError> {
        let config_guard = self.config.read().map_err(|_| AppError::ConfigError(anyhow::anyhow!("Config lock poisoned")))?;
        let root_search_path = validate_path_access(&params.path, &config_guard, true)?; // Corrected: params.path
        debug!(search_root = %root_search_path.display(), pattern = %params.pattern, "Searching files by name");
        
        let search_operation_config = config_guard.clone(); // Clone Arc for async block
        drop(config_guard); // Release lock

        let search_operation = async move {
            let mut matches = Vec::new();
            let pattern_lower = params.pattern.to_lowercase();
            let mut dirs_to_visit = vec![root_search_path.clone()];
            let files_root_clone = search_operation_config.read().unwrap().files_root.clone(); // Read inside async

            while let Some(current_dir) = dirs_to_visit.pop() {
                let mut read_dir = match fs::read_dir(&current_dir).await { // Corrected: &current_dir
                    Ok(rd) => rd,
                    Err(e) => {
                        warn!(dir = %current_dir.display(), error = %e, "Could not read directory during search_files");
                        continue;
                    }
                };

                while let Some(entry_res) = read_dir.next_entry().await.map_err(AppError::from)? { // Propagate IO errors
                    let entry = entry_res;
                    let entry_path = entry.path();
                    if entry_path.file_name().unwrap_or_default().to_string_lossy().to_lowercase().contains(&pattern_lower) {
                        if let Ok(relative_path) = entry_path.strip_prefix(&files_root_clone) {
                            matches.push(relative_path.to_string_lossy().into_owned());
                        } else {
                            matches.push(entry_path.to_string_lossy().into_owned());
                        }
                    }
                    if entry_path.is_dir() {
                         // Re-check with config inside async block
                        if validate_path_access(entry_path.to_str().unwrap_or_default(), &search_operation_config.read().unwrap(), true).is_ok() {
                            dirs_to_visit.push(entry_path);
                        }
                    }
                }
            }
            matches.sort();
            Ok(matches)
        };

        let timeout_duration = Duration::from_millis(params.timeout_ms.unwrap_or(FILE_OPERATION_TIMEOUT_MS));
        match timeout(timeout_duration, search_operation).await {
            Ok(Ok(matches)) => Ok(SearchFilesResult {
                path: params.path.clone(),
                pattern: params.pattern.clone(),
                matches,
                timed_out: false,
            }),
            Ok(Err(e)) => Err(e),
            Err(_) => Ok(SearchFilesResult {
                path: params.path.clone(),
                pattern: params.pattern.clone(),
                matches: vec![],
                timed_out: true,
            }),
        }
    }
    
    #[instrument(skip(self, params), fields(path = %params.path))]
    pub async fn get_file_info(&self, params: &GetFileInfoParams) -> Result<FileInfoResult, AppError> {
        let config_guard = self.config.read().map_err(|_| AppError::ConfigError(anyhow::anyhow!("Config lock poisoned")))?;
        let path = validate_path_access(&params.path, &config_guard, true)?; // Corrected: params.path
        drop(config_guard);
        debug!(info_path = %path.display(), "Getting file info");
        let metadata = fs::metadata(&path).await?;

        let to_iso = |st: Result<std::time::SystemTime, _>| {
            st.ok()
                .map(chrono::DateTime::<chrono::Utc>::from)
                .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        };

        let permissions_octal = if cfg!(unix) {
            use std::os::unix::fs::PermissionsExt; // Moved use statement inside block
            Some(format!("{:03o}", metadata.permissions().mode() & 0o777))
        } else {
            None 
        };

        Ok(FileInfoResult {
            path: params.path.clone(),
            size: metadata.len(),
            is_dir: metadata.is_dir(),
            is_file: metadata.is_file(),
            modified_iso: to_iso(metadata.modified()),
            created_iso: to_iso(metadata.created()),
            accessed_iso: to_iso(metadata.accessed()),
            permissions_octal,
        })
    }
}

use std::time::Duration;