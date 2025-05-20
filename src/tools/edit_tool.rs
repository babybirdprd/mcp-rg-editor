use crate::config::Config;
use crate::error::AppError;
use crate::tools::filesystem_tool::{FilesystemManager, ReadFileParams, WriteFileParams, WriteMode};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{debug, instrument};

#[derive(Debug, Deserialize)]
pub struct EditBlockParams {
    pub file_path: String,
    pub old_string: String,
    pub new_string: String,
    #[serde(default = "default_one")]
    pub expected_replacements: usize,
}
fn default_one() -> usize { 1 }

#[derive(Debug, Serialize)]
pub struct EditBlockResult {
    pub file_path: String,
    pub replacements_made: usize,
    pub message: String,
}

#[derive(Debug)]
pub struct EditManager {
    config: Arc<Config>,
    filesystem_manager: Arc<FilesystemManager>,
}

impl EditManager {
    pub fn new(config: Arc<Config>, filesystem_manager: Arc<FilesystemManager>) -> Self {
        Self { config, filesystem_manager }
    }

    #[instrument(skip(self, params), fields(file_path = %params.file_path))]
    pub async fn edit_block(&self, params: &EditBlockParams) -> Result<EditBlockResult, AppError> {
        debug!("Editing block in file: {}", params.file_path);

        if params.old_string.is_empty() {
            return Err(AppError::EditError("Search string (old_string) cannot be empty.".to_string()));
        }

        let read_params = ReadFileParams {
            path: params.file_path.clone(),
            offset: 0,
            length: Some(self.config.file_read_line_limit * 100), // Read a large chunk, but not unlimited
        };
        let file_content_result = self.filesystem_manager.read_file(&read_params).await?;
        let mut content = file_content_result.content;

        // Count occurrences
        let occurrences: Vec<_> = content.match_indices(¶ms.old_string).collect();
        let actual_occurrences = occurrences.len();

        if actual_occurrences == 0 {
             // Try a simple similarity check for error reporting
            let similarity = strsim::jaro_winkler(&content, ¶ms.old_string);
            return Err(AppError::EditError(format!(
                "Search string not found. Closest similarity of search string to entire content: {:.2}%.",
                similarity * 100.0
            )));
        }

        if params.expected_replacements > 0 && actual_occurrences != params.expected_replacements {
            return Err(AppError::EditError(format!(
                "Expected {} occurrences, but found {}. Please verify and adjust 'expected_replacements'.",
                params.expected_replacements, actual_occurrences
            )));
        }
        
        // Perform replacement
        // If expected_replacements is 0, it means replace all occurrences found.
        let num_to_replace = if params.expected_replacements == 0 {
            actual_occurrences
        } else {
            params.expected_replacements
        };

        if num_to_replace == 1 && actual_occurrences == 1 { // Common case: replace first and only
            content = content.replacen(¶ms.old_string, ¶ms.new_string, 1);
        } else { // Replace all matched (up to num_to_replace if specified, or all if expected_replacements was 0)
            content = content.replace(¶ms.old_string, ¶ms.new_string);
        }
        
        let replacements_made = if params.expected_replacements == 0 { actual_occurrences } else { actual_occurrences.min(params.expected_replacements) };


        let write_params = WriteFileParams {
            path: params.file_path.clone(),
            content,
            mode: WriteMode::Rewrite,
        };
        self.filesystem_manager.write_file(&write_params).await?;

        Ok(EditBlockResult {
            file_path: params.file_path.clone(),
            replacements_made,
            message: format!("Successfully made {} replacement(s).", replacements_made),
        })
    }
}