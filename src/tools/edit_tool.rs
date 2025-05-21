use crate::config::Config;
use crate::error::AppError;
use crate::tools::filesystem_tool::{FilesystemManager, ReadFileParams, WriteFileParams, WriteMode};
use crate::utils::audit_logger::AuditLogger; // Added for logging
use crate::utils::fuzzy_search_logger::{FuzzySearchLogger, FuzzySearchLogEntry};
use crate::utils::line_ending_handler::{detect_line_ending, normalize_line_endings}; // Removed LineEndingStyle as it's used internally
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, RwLock as StdRwLock};
use tracing::{debug, instrument, warn};
use std::time::Instant;
use chrono::Utc;
use diff; // Ensure this crate is in Cargo.toml

#[derive(Debug, Deserialize)]
pub struct EditBlockParams {
    pub file_path: String,
    pub old_string: String,
    pub new_string: String,
    #[serde(default = "default_one_usize")]
    pub expected_replacements: usize,
}
fn default_one_usize() -> usize { 1 }

#[derive(Debug, Serialize)]
pub struct EditBlockResult {
    pub file_path: String,
    pub replacements_made: usize,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fuzzy_match_details: Option<FuzzyMatchDetails>,
}

#[derive(Debug, Serialize)]
pub struct FuzzyMatchDetails {
    pub similarity_percent: f64,
    pub execution_time_ms: f64,
    pub diff_highlight: String,
    pub log_path_suggestion: String,
}

const FUZZY_SIMILARITY_THRESHOLD: f64 = 0.7;

#[derive(Debug)]
pub struct EditManager {
    config: Arc<StdRwLock<Config>>,
    filesystem_manager: Arc<FilesystemManager>,
    fuzzy_logger: Arc<FuzzySearchLogger>, // Changed to Arc<FuzzySearchLogger>
    _audit_logger: Arc<AuditLogger>, // Keep audit logger if needed for other methods
}

impl EditManager {
    pub fn new(
        config: Arc<StdRwLock<Config>>, 
        filesystem_manager: Arc<FilesystemManager>,
        audit_logger: Arc<AuditLogger> // Accept AuditLogger
    ) -> Self {
        let fuzzy_logger = Arc::new(FuzzySearchLogger::new(config.clone()));
        Self { 
            config, 
            filesystem_manager, 
            fuzzy_logger,
            _audit_logger: audit_logger,
        }
    }

    #[instrument(skip(self, params), fields(file_path = %params.file_path))]
    pub async fn edit_block(&self, params: &EditBlockParams) -> Result<EditBlockResult, AppError> {
        debug!("Editing block in file: {}", params.file_path);

        if params.old_string.is_empty() {
            return Err(AppError::EditError("Search string (old_string) cannot be empty.".to_string()));
        }

        let file_path_obj = Path::new(&params.file_path);
        let file_extension = file_path_obj.extension().unwrap_or_default().to_string_lossy().to_lowercase();
        
        let config_read_guard = self.config.read().map_err(|_| AppError::ConfigError(anyhow::anyhow!("Config lock poisoned")))?;

        let read_params = ReadFileParams {
            path: params.file_path.clone(),
            is_url: false,
            offset: 0,
            length: Some(usize::MAX), // Read full file for editing
        };
        let file_content_result = self.filesystem_manager.read_file(&read_params).await?;
        let original_content = file_content_result.text_content.ok_or_else(|| AppError::EditError("File content is not text or could not be read.".to_string()))?;

        let file_line_ending = detect_line_ending(&original_content);
        debug!(?file_line_ending, "Detected file line ending");

        let normalized_old_string = normalize_line_endings(&params.old_string, file_line_ending);
        let normalized_new_string = normalize_line_endings(&params.new_string, file_line_ending);

        let occurrences: Vec<_> = original_content.match_indices(&normalized_old_string).collect();
        let actual_occurrences = occurrences.len();

        if params.expected_replacements > 0 && actual_occurrences == params.expected_replacements {
            debug!(count = actual_occurrences, "Exact match count matches expected. Proceeding with replacement.");
            let new_content = original_content.replace(&normalized_old_string, &normalized_new_string);
            
            let write_params = WriteFileParams {
                path: params.file_path.clone(),
                content: new_content,
                mode: WriteMode::Rewrite,
            };
            self.filesystem_manager.write_file(&write_params).await?;

            return Ok(EditBlockResult {
                file_path: params.file_path.clone(),
                replacements_made: actual_occurrences,
                message: format!("Successfully applied {} exact replacement(s).", actual_occurrences),
                fuzzy_match_details: None,
            });
        }
        
        if params.expected_replacements == 0 && actual_occurrences > 0 {
            debug!(count = actual_occurrences, "Expected 0 (replace all) and found occurrences. Proceeding with replacement.");
            let new_content = original_content.replace(&normalized_old_string, &normalized_new_string);
            let write_params = WriteFileParams {
                path: params.file_path.clone(),
                content: new_content,
                mode: WriteMode::Rewrite,
            };
            self.filesystem_manager.write_file(&write_params).await?;
            return Ok(EditBlockResult {
                file_path: params.file_path.clone(),
                replacements_made: actual_occurrences,
                message: format!("Successfully applied {} exact replacement(s) (all occurrences).", actual_occurrences),
                fuzzy_match_details: None,
            });
        }

        if actual_occurrences > 0 && params.expected_replacements > 0 && actual_occurrences != params.expected_replacements {
             return Err(AppError::EditError(format!(
                "Expected {} occurrences but found {}. Please verify 'old_string' for uniqueness or adjust 'expected_replacements'. If you want to replace all {} occurrences, set expected_replacements to {}.",
                params.expected_replacements, actual_occurrences, actual_occurrences, actual_occurrences
            )));
        }
        
        debug!("No exact match or count mismatch for specific replacement. Attempting fuzzy search.");
        let fuzzy_start_time = Instant::now();
        
        let (best_match_value, similarity) = find_best_fuzzy_match(&original_content, &normalized_old_string);
        let fuzzy_execution_time_ms = fuzzy_start_time.elapsed().as_secs_f64() * 1000.0;

        let diff_highlight = highlight_differences(&normalized_old_string, &best_match_value);
        let char_code_data = get_character_code_data(&normalized_old_string, &best_match_value);

        let log_entry = FuzzySearchLogEntry {
            timestamp: Utc::now(),
            search_text: params.old_string.clone(),
            found_text: best_match_value.clone(),
            similarity,
            execution_time_ms: fuzzy_execution_time_ms,
            exact_match_count: actual_occurrences,
            expected_replacements: params.expected_replacements,
            fuzzy_threshold: FUZZY_SIMILARITY_THRESHOLD,
            below_threshold: similarity < FUZZY_SIMILARITY_THRESHOLD,
            diff: diff_highlight.clone(),
            search_length: params.old_string.len(),
            found_length: best_match_value.len(),
            file_extension: file_extension.to_string(),
            character_codes: char_code_data.report,
            unique_character_count: char_code_data.unique_count,
            diff_length: char_code_data.diff_length,
        };
        
        // Clone Arc for the spawned task
        let logger_clone = self.fuzzy_logger.clone();
        tokio::spawn(async move {
            if let Err(e) = logger_clone.log(&log_entry).await { // Call log on Arc<FuzzySearchLogger>
                 tracing::error!("Failed to log fuzzy search entry: {}", e);
            }
        });

        let fuzzy_details = FuzzyMatchDetails {
            similarity_percent: similarity * 100.0,
            execution_time_ms: fuzzy_execution_time_ms,
            diff_highlight: diff_highlight.clone(),
            log_path_suggestion: config_read_guard.fuzzy_search_log_file.display().to_string(),
        };
        drop(config_read_guard);

        if similarity >= FUZZY_SIMILARITY_THRESHOLD {
            warn!(similarity, "Fuzzy match found, but not applied automatically.");
            Ok(EditBlockResult {
                file_path: params.file_path.clone(),
                replacements_made: 0,
                message: format!(
                    "Exact match not found. Found a similar text with {:.2}% similarity (search took {:.2}ms). Please review the differences and provide the exact text from the file if you wish to replace it.",
                    similarity * 100.0, fuzzy_execution_time_ms
                ),
                fuzzy_match_details: Some(fuzzy_details),
            })
        } else {
            Err(AppError::EditError(format!(
                "Search string not found. Closest fuzzy match had {:.2}% similarity (below threshold of {}%). Diff: {}",
                similarity * 100.0, FUZZY_SIMILARITY_THRESHOLD * 100.0, diff_highlight
            )))
        }
    }
}

fn find_best_fuzzy_match(text: &str, query: &str) -> (String, f64) {
    if text.is_empty() || query.is_empty() {
        return ("".to_string(), 0.0);
    }

    let mut best_similarity = 0.0;
    let mut best_match_str = ""; // Use &str for slices

    let text_chars: Vec<char> = text.chars().collect();
    // query_chars is not used directly in this revised logic, but length is.
    let text_len = text_chars.len();
    let query_len = query.chars().count(); // Use chars().count() for char length
    
    if query_len == 0 { return ("".to_string(), 0.0); }

    let min_window_len = std::cmp::max(1, query_len.saturating_sub(query_len / 4));
    let max_window_len = std::cmp::min(text_len, query_len + query_len / 4);

    for window_len_chars in min_window_len..=max_window_len {
        if window_len_chars > text_len { continue; }
        for i in 0..=(text_len - window_len_chars) {
            // Get byte indices for slicing based on char indices
            let start_byte_idx = text.char_indices().nth(i).map(|(idx, _)| idx).unwrap_or(0);
            let end_byte_idx = text.char_indices().nth(i + window_len_chars).map(|(idx, _)| idx).unwrap_or_else(|| text.len());
            
            let window_str_slice = &text[start_byte_idx..end_byte_idx];
            
            let current_similarity = strsim::jaro_winkler(window_str_slice, query);

            if current_similarity > best_similarity {
                best_similarity = current_similarity;
                best_match_str = window_str_slice;
            }
            // Optimization: if very high similarity, assume it's the best we'll find
            if best_similarity > 0.999 { // Increased threshold for early exit
                return (best_match_str.to_string(), best_similarity);
            }
        }
    }
    (best_match_str.to_string(), best_similarity)
}

fn highlight_differences(expected: &str, actual: &str) -> String {
    let diff_results = diff::chars(expected, actual);
    let mut result = String::new();
    for d_res in diff_results {
        match d_res {
            diff::Result::Left(l) => result.push_str(&format!("{{-{}-}}", l)), // l is char, format needs &str
            diff::Result::Both(l, _) => result.push(l), // l is char
            diff::Result::Right(r) => result.push_str(&format!("{{+{}+}}", r)), // r is char
        }
    }
    result
}

struct CharCodeData {
    report: String,
    unique_count: usize,
    diff_length: usize,
}

fn get_character_code_data(expected: &str, actual: &str) -> CharCodeData {
    use std::collections::HashMap;

    let mut prefix_len = 0;
    let min_char_len = std::cmp::min(expected.chars().count(), actual.chars().count());
    
    let mut expected_chars = expected.chars();
    let mut actual_chars = actual.chars();

    for _ in 0..min_char_len {
        if expected_chars.next() == actual_chars.next() {
            prefix_len +=1;
        } else {
            break;
        }
    }
    
    // Reset iterators for suffix
    expected_chars = expected.chars().rev();
    actual_chars = actual.chars().rev();
    let mut suffix_len = 0;
    for _ in 0..(min_char_len - prefix_len) {
         if expected_chars.next() == actual_chars.next() {
            suffix_len +=1;
        } else {
            break;
        }
    }
    
    let expected_diff_str: String = expected.chars().skip(prefix_len).take(expected.chars().count() - prefix_len - suffix_len).collect();
    let actual_diff_str: String = actual.chars().skip(prefix_len).take(actual.chars().count() - prefix_len - suffix_len).collect();


    let mut char_codes: HashMap<u32, usize> = HashMap::new();
    let full_diff_str = format!("{}{}", expected_diff_str, actual_diff_str);

    for ch in full_diff_str.chars() {
        *char_codes.entry(ch as u32).or_insert(0) += 1;
    }
    
    let mut report_parts: Vec<String> = char_codes
        .iter()
        .map(|(&code, &count)| {
            let char_display = std::char::from_u32(code)
                .map(|c| if c.is_control() || (c.is_whitespace() && c != ' ') { format!("\\x{:02x}", code) } else { c.to_string() })
                .unwrap_or_else(|| format!("\\u{{{:x}}}", code));
            format!("{}:{}[{}]", code, count, char_display)
        })
        .collect();
    report_parts.sort();

    CharCodeData {
        report: report_parts.join(","),
        unique_count: char_codes.len(),
        diff_length: full_diff_str.chars().count(),
    }
}