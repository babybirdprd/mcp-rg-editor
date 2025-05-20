use crate::config::Config;
use crate::error::AppError;
use crate::utils::path_utils::{validate_path_access, validate_parent_path_access};
use crate::utils::line_ending_handler::{detect_line_ending, normalize_line_endings, LineEndingStyle};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::time::timeout;
use tracing::{debug, instrument, warn};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};


// --- Schemas for parameters ---
#[derive(Debug, Deserialize)]
pub struct ReadFileParams {
    pub path: String,
    #[serde(default)]
    pub is_url: bool,
    #[serde(default)]
    pub offset: usize, // Line offset for text files
    pub length: Option<usize>, // Max lines for text files, uses config default if None
}

#[derive(Debug, Deserialize)]
pub struct ReadMultipleFilesParams {
    pub paths: Vec<String>,
    // is_url could be added per path if needed, or assume all are local for now
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
    pub pattern: String, // Simple substring match
    #[serde(rename = "timeoutMs")]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct GetFileInfoParams {
    pub path: String,
}

// --- Schemas for results ---
#[derive(Debug, Serialize)]
pub struct FileContent {
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_content: Option<String>, // For text files
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_data_base64: Option<String>, // For images
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
    pub permissions_octal: Option<String>, // e.g., "755"
}

// --- Filesystem Operations ---
#[derive(Debug)]
pub struct FilesystemManager {
    config: Arc<Config>,
    // For URL reading, we can use a shared client
    http_client: reqwest::Client,
}

const URL_FETCH_TIMEOUT_MS: u64 = 30000; // 30 seconds
const FILE_OPERATION_TIMEOUT_MS: u64 = 30000; // 30 seconds for local file ops

fn is_image_mime(mime_type: &str) -> bool {
    mime_type.starts_with("image/")
}

impl FilesystemManager {
    pub fn new(config: Arc<Config>) -> Self {
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
                lines_read: Some(lines.len()), // URLs are read fully
                total_lines: Some(lines.len()),
                truncated: Some(false),
                error: None,
            })
        }
    }

    #[instrument(skip(self, params), fields(path = %params.path, is_url = %params.is_url))]
    pub async fn read_file(&self, params: &ReadFileParams) -> Result<FileContent, AppError> {
        if params.is_url {
            return self.read_file_from_url(¶ms.path).await;
        }

        let path = validate_path_access(¶ms.path, &self.config, true)?;
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
            let read_limit = params.length.unwrap_or(self.config.file_read_line_limit);
            
            while let Some(line_res) = lines_iter.next_line().await? {
                total_lines_count += 1;
                if current_line_idx >= params.offset && content_vec.len() < read_limit {
                    content_vec.push(line_res);
                }
                current_line_idx += 1;
                if content_vec.len() >= read_limit && params.offset + content_vec.len() < total_lines_count {
                    // We've read enough lines and there are more lines available in the file
                    break; 
                }
            }
            
            let lines_read_count = content_vec.len();
            let text_content = content_vec.join("\n");
            let is_truncated = params.offset > 0 || (lines_read_count == read_limit && params.offset + lines_read_count < total_lines_count);

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
        for path_str in ¶ms.paths {
            // Assuming all paths are local for now. If URLs are needed, logic would need to adapt.
            let read_params = ReadFileParams {
                path: path_str.clone(),
                is_url: false, // Modify if URLs can be mixed
                offset: 0,
                length: Some(self.config.file_read_line_limit), // Use default limit for each
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
        let path = validate_parent_path_access(¶ms.path, &self.config)?;
        debug!(write_path = %path.display(), mode = ?params.mode, "Writing file");

        let lines: Vec<&str> = params.content.lines().collect();
        if lines.len() > self.config.file_write_line_limit {
            return Err(AppError::EditError(format!(
                "Content exceeds line limit of {}. Received {} lines. Please send content in smaller chunks.",
                self.config.file_write_line_limit,
                lines.len()
            )));
        }
        
        // Preserve original line endings if appending to existing file, otherwise use system default
        let final_content = if params.mode == WriteMode::Append && path.exists() {
            let existing_content_str = fs::read_to_string(&path).await.unwrap_or_default();
            let detected_ending = detect_line_ending(&existing_content_str);
            normalize_line_endings(¶ms.content, detected_ending)
        } else {
            let system_ending = if cfg!(windows) { LineEndingStyle::CrLf } else { LineEndingStyle::Lf };
            normalize_line_endings(¶ms.content, system_ending)
        };


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
        let path = validate_parent_path_access(¶ms.path, &self.config)?;
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
        let path = validate_path_access(¶ms.path, &self.config, true)?;
        debug!(list_dir_path = %path.display(), "Listing directory");
        let mut entries = Vec::new();
        let mut read_dir = fs::read_dir(&path).await?;
        while let Some(entry_res) = read_dir.next_entry().await? {
            let entry_path = entry_res.path();
            let entry_name = entry_res.file_name().to_string_lossy().to_string();
            let prefix = if entry_path.is_dir() { "[DIR] " } else { "[FILE]" };
            entries.push(format!("{} {}", prefix, entry_name));
        }
        entries.sort(); // For consistent output
        Ok(ListDirectoryResult {
            path: params.path.clone(),
            entries,
        })
    }

    #[instrument(skip(self, params), fields(source = %params.source, dest = %params.destination))]
    pub async fn move_file(&self, params: &MoveFileParams) -> Result<FileOperationResult, AppError> {
        let source_path = validate_path_access(¶ms.source, &self.config, true)?;
        let dest_path = validate_parent_path_access(¶ms.destination, &self.config)?; 
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
        let root_search_path = validate_path_access(¶ms.path, &self.config, true)?;
        debug!(search_root = %root_search_path.display(), pattern = %params.pattern, "Searching files by name");
        
        let search_operation = async {
            let mut matches = Vec::new();
            let pattern_lower = params.pattern.to_lowercase();
            let mut dirs_to_visit = vec![root_search_path.clone()];

            while let Some(current_dir) = dirs_to_visit.pop() {
                let mut read_dir = match fs::read_dir(¤t_dir).await {
                    Ok(rd) => rd,
                    Err(e) => {
                        warn!(dir = %current_dir.display(), error = %e, "Could not read directory during search_files");
                        continue;
                    }
                };

                while let Some(entry_res) = read_dir.next_entry().await? {
                    let entry = entry_res;
                    let entry_path = entry.path();
                    if entry_path.file_name().unwrap_or_default().to_string_lossy().to_lowercase().contains(&pattern_lower) {
                        // Return path relative to files_root for brevity and security
                        if let Ok(relative_path) = entry_path.strip_prefix(&self.config.files_root) {
                            matches.push(relative_path.to_string_lossy().into_owned());
                        } else {
                            // Fallback to absolute if not under files_root (should not happen with validation)
                            matches.push(entry_path.to_string_lossy().into_owned());
                        }
                    }
                    if entry_path.is_dir() {
                        // Ensure subdirectory is also allowed before adding to visit list
                        if validate_path_access(entry_path.to_str().unwrap_or_default(), &self.config, true).is_ok() {
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
            Ok(Err(e)) => Err(e), // Propagate AppError from search_operation
            Err(_) => Ok(SearchFilesResult { // Timeout occurred
                path: params.path.clone(),
                pattern: params.pattern.clone(),
                matches: vec![],
                timed_out: true,
            }),
        }
    }
    
    #[instrument(skip(self, params), fields(path = %params.path))]
    pub async fn get_file_info(&self, params: &GetFileInfoParams) -> Result<FileInfoResult, AppError> {
        let path = validate_path_access(¶ms.path, &self.config, true)?;
        debug!(info_path = %path.display(), "Getting file info");
        let metadata = fs::metadata(&path).await?;

        let to_iso = |st: Result<std::time::SystemTime, _>| {
            st.ok()
                .map(chrono::DateTime::<chrono::Utc>::from)
                .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        };

        let permissions_octal = if cfg!(unix) {
            use std::os::unix::fs::PermissionsExt;
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