use crate::config::Config;
use crate::error::AppError;
use crate::utils::path_utils::{validate_path_access, validate_parent_path_access};
use serde::{Deserialize, Serialize};
use std::fs::Metadata;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{debug, instrument};

// --- Schemas for parameters ---
#[derive(Debug, Deserialize)]
pub struct ReadFileParams {
    pub path: String,
    #[serde(default)]
    pub offset: usize,
    pub length: Option<usize>, // uses config default if None
}

#[derive(Debug, Deserialize)]
pub struct WriteFileParams {
    pub path: String,
    pub content: String,
    #[serde(default = "default_rewrite_mode")]
    pub mode: WriteMode,
}
fn default_rewrite_mode() -> WriteMode { WriteMode::Rewrite }

#[derive(Debug, Deserialize, PartialEq)]
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
}

#[derive(Debug, Deserialize)]
pub struct GetFileInfoParams {
    pub path: String,
}

// --- Schemas for results ---
#[derive(Debug, Serialize)]
pub struct FileContentResult {
    pub path: String,
    pub content: String,
    pub lines_read: usize,
    pub total_lines: Option<usize>, // None if not applicable (e.g. error)
    pub truncated: bool,
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
}

#[derive(Debug, Serialize)]
pub struct FileInfoResult {
    pub path: String,
    pub size: u64,
    pub is_dir: bool,
    pub is_file: bool,
    pub modified_ms: Option<u128>, // SystemTime to milliseconds since UNIX_EPOCH
    pub created_ms: Option<u128>,
    pub permissions: Option<String>, // e.g. "rwxr-xr-x"
}

// --- Filesystem Operations ---
#[derive(Debug)]
pub struct FilesystemManager {
    config: Arc<Config>,
}

impl FilesystemManager {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }

    #[instrument(skip(self, params), fields(path = %params.path))]
    pub async fn read_file(&self, params: &ReadFileParams) -> Result<FileContentResult, AppError> {
        let path = validate_path_access(¶ms.path, &self.config, true)?;
        debug!("Reading file: {:?}", path);

        let file = fs::File::open(&path).await?;
        let reader = BufReader::new(file);
        let mut lines = reader.lines();

        let mut content_vec = Vec::new();
        let mut current_line_idx = 0;
        let mut total_lines = 0;
        let read_limit = params.length.unwrap_or(self.config.file_read_line_limit);
        
        while let Some(line_res) = lines.next_line().await? {
            total_lines += 1;
            if current_line_idx >= params.offset && content_vec.len() < read_limit {
                content_vec.push(line_res);
            }
            current_line_idx += 1;
             if content_vec.len() >= read_limit && params.offset + content_vec.len() < total_lines {
                break; // Stop if we've read enough lines and there are more lines available
            }
        }
        
        let lines_read = content_vec.len();
        let content = content_vec.join("\n");
        let truncated = params.offset > 0 || (lines_read == read_limit && params.offset + lines_read < total_lines);


        Ok(FileContentResult {
            path: params.path.clone(),
            content,
            lines_read,
            total_lines: Some(total_lines),
            truncated,
        })
    }

    #[instrument(skip(self, params), fields(path = %params.path, mode = ?params.mode))]
    pub async fn write_file(&self, params: &WriteFileParams) -> Result<FileOperationResult, AppError> {
        let path = validate_parent_path_access(¶ms.path, &self.config)?;
        debug!("Writing file: {:?}, mode: {:?}", path, params.mode);

        let lines: Vec<&str> = params.content.lines().collect();
        if lines.len() > self.config.file_write_line_limit {
            return Err(AppError::EditError(format!(
                "Content exceeds line limit of {}. Received {} lines.",
                self.config.file_write_line_limit,
                lines.len()
            )));
        }

        let mut file = match params.mode {
            WriteMode::Rewrite => fs::File::create(&path).await?,
            WriteMode::Append => fs::OpenOptions::new().append(true).create(true).open(&path).await?,
        };
        file.write_all(params.content.as_bytes()).await?;
        file.flush().await?;

        Ok(FileOperationResult {
            success: true,
            path: params.path.clone(),
            message: format!("Successfully {} file.", if params.mode == WriteMode::Append {"appended to"} else {"wrote to"}),
        })
    }
    
    #[instrument(skip(self, params), fields(path = %params.path))]
    pub async fn create_directory(&self, params: &CreateDirectoryParams) -> Result<FileOperationResult, AppError> {
        let path = validate_parent_path_access(¶ms.path, &self.config)?;
        debug!("Creating directory: {:?}", path);
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
        debug!("Listing directory: {:?}", path);
        let mut entries = Vec::new();
        let mut read_dir = fs::read_dir(&path).await?;
        while let Some(entry) = read_dir.next_entry().await? {
            let entry_path = entry.path();
            let entry_name = entry.file_name().to_string_lossy().to_string();
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
        let dest_path = validate_parent_path_access(¶ms.destination, &self.config)?; // Parent of dest must be valid
        debug!("Moving {:?} to {:?}", source_path, dest_path);
        fs::rename(&source_path, &dest_path).await?;
        Ok(FileOperationResult {
            success: true,
            path: params.destination.clone(), // Report destination path
            message: format!("Successfully moved {} to {}.", params.source, params.destination),
        })
    }

    #[instrument(skip(self, params), fields(path = %params.path, pattern = %params.pattern))]
    pub async fn search_files(&self, params: &SearchFilesParams) -> Result<SearchFilesResult, AppError> {
        let root_search_path = validate_path_access(¶ms.path, &self.config, true)?;
        debug!("Searching files in {:?} for pattern '{}'", root_search_path, params.pattern);
        let mut matches = Vec::new();
        let pattern_lower = params.pattern.to_lowercase();

        let mut dirs_to_visit = vec![root_search_path.clone()];

        while let Some(current_dir) = dirs_to_visit.pop() {
            let mut read_dir = match fs::read_dir(¤t_dir).await {
                Ok(rd) => rd,
                Err(e) => {
                    tracing::warn!("Could not read directory {}: {}", current_dir.display(), e);
                    continue;
                }
            };

            while let Some(entry_res) = read_dir.next_entry().await? {
                let entry = entry_res;
                let entry_path = entry.path();
                if entry_path.file_name().unwrap_or_default().to_string_lossy().to_lowercase().contains(&pattern_lower) {
                     if let Ok(relative_path) = entry_path.strip_prefix(&self.config.files_root) {
                        matches.push(relative_path.to_string_lossy().into_owned());
                    } else {
                        matches.push(entry_path.to_string_lossy().into_owned()); // Fallback to absolute
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
        Ok(SearchFilesResult {
            path: params.path.clone(),
            pattern: params.pattern.clone(),
            matches,
        })
    }
    
    #[instrument(skip(self, params), fields(path = %params.path))]
    pub async fn get_file_info(&self, params: &GetFileInfoParams) -> Result<FileInfoResult, AppError> {
        let path = validate_path_access(¶ms.path, &self.config, true)?;
        debug!("Getting file info for: {:?}", path);
        let metadata = fs::metadata(&path).await?;

        let modified_ms = metadata.modified().ok()
            .and_then(|st| st.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis());
        
        let created_ms = metadata.created().ok()
            .and_then(|st| st.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis());

        let permissions = if cfg!(unix) {
            use std::os::unix::fs::PermissionsExt;
            Some(format!("{:o}", metadata.permissions().mode() & 0o777))
        } else {
            None // Permissions mode is Unix-specific
        };

        Ok(FileInfoResult {
            path: params.path.clone(),
            size: metadata.len(),
            is_dir: metadata.is_dir(),
            is_file: metadata.is_file(),
            modified_ms,
            created_ms,
            permissions,
        })
    }
}