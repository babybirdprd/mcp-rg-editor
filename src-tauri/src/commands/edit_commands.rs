use crate::config::Config;
use crate::error::AppError;
use crate::utils::audit_logger::audit_log;
use crate::utils::fuzzy_search_logger::{FuzzySearchLogger, FuzzySearchLogEntry};
use crate::utils::line_ending_handler::{detect_line_ending, normalize_line_endings, LineEndingStyle};
use crate::utils::path_utils::validate_and_normalize_path;
// For internal file reading/writing logic, similar to filesystem_commands
use crate::commands::filesystem_commands::{FileContent, WriteMode as InternalWriteMode};


use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock as StdRwLock};
use tauri::{AppHandle, Manager, State};
use tracing::{debug, instrument, warn, error};
use std::time::Instant;
use chrono::Utc;
use diff; // For diff highlighting

// --- Request Structs ---
#[derive(Debug, Deserialize, Serialize)] // Added Serialize for audit log
pub struct EditBlockParams {
    pub file_path: String,
    pub old_string: String,
    pub new_string: String,
    #[serde(default = "default_one_usize")]
    pub expected_replacements: usize,
}
fn default_one_usize() -> usize { 1 }

// --- Response Structs ---
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

const FUZZY_SIMILARITY_THRESHOLD: f64 = 0.7; // From original mcp-rg-editor

// Internal helper to read file content for editing
async fn read_file_for_edit(
    app_handle: &AppHandle,
    file_path_str: &str,
    config_guard: &StdRwLockReadGuard<'_, Config>, // Borrow config_guard
) -> Result<(String, PathBuf, LineEndingStyle), AppError> {
    let path = validate_and_normalize_path(file_path_str, config_guard, true, false)?;
    if !app_handle.fs_scope().is_allowed(&path) {
        return Err(AppError::PathNotAllowed(format!("Read access to {} denied by FS scope.", path.display())));
    }
    let content_bytes = app_handle.fs().read_binary_file(&path).await
        .map_err(|e| AppError::PluginError { plugin: "fs".to_string(), message: format!("Failed to read file {}: {}", path.display(), e) })?;

    // Try to decode as UTF-8. This is a simplification; robust handling might involve checking BOM, etc.
    let original_content = String::from_utf8(content_bytes)
        .map_err(|e| AppError::EditError(format!("File {} is not valid UTF-8: {}", path.display(), e)))?;

    let line_ending = detect_line_ending(&original_content);
    Ok((original_content, path, line_ending))
}

// Internal helper to write file content after editing
async fn write_file_after_edit(
    app_handle: &AppHandle,
    file_path_obj: &Path, // Already validated path
    content: String,
    _config_guard: &StdRwLockReadGuard<'_, Config>, // Borrow config_guard
) -> Result<(), AppError> {
    if !app_handle.fs_scope().is_allowed(file_path_obj) {
        return Err(AppError::PathNotAllowed(format!("Write access to {} denied by FS scope.", file_path_obj.display())));
    }
    app_handle.fs().write_text_file(file_path_obj, content).await
        .map_err(|e| AppError::PluginError { plugin: "fs".to_string(), message: format!("Failed to write file {}: {}", file_path_obj.display(), e) })
}


#[tauri::command(async)]
#[instrument(skip(app_handle, config_state, audit_logger_state, fuzzy_logger_state, params), fields(file_path = %params.file_path))]
pub async fn edit_block_command(
    app_handle: AppHandle,
    config_state: State<'_, Arc<StdRwLock<Config>>>,
    audit_logger_state: State<'_, Arc<crate::utils::audit_logger::AuditLogger>>,
    fuzzy_logger_state: State<'_, Arc<FuzzySearchLogger>>,
    params: EditBlockParams,
) -> Result<EditBlockResult, AppError> {
    audit_log(&audit_logger_state, "edit_block", &serde_json::to_value(params)?).await;
    debug!("Editing block in file: {}", params.file_path);

    if params.old_string.is_empty() {
        return Err(AppError::EditError("Search string (old_string) cannot be empty.".to_string()));
    }

    let config_read_guard = config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock error: {}",e)))?;

    let (original_content, validated_path, file_line_ending) =
        read_file_for_edit(&app_handle, params.file_path, &config_read_guard).await?;

    let file_extension = validated_path.extension().unwrap_or_default().to_string_lossy().to_lowercase();
    debug!(?file_line_ending, "Detected file line ending for edit");

    let normalized_old_string = normalize_line_endings(params.old_string, file_line_ending);
    let normalized_new_string = normalize_line_endings(params.new_string, file_line_ending);

    let occurrences: Vec<_> = original_content.match_indices(&normalized_old_string).collect();
    let actual_occurrences = occurrences.len();

    if params.expected_replacements > 0 && actual_occurrences == params.expected_replacements {
        debug!(count = actual_occurrences, "Exact match count matches expected. Proceeding with replacement.");
        let new_content = original_content.replace(&normalized_old_string, &normalized_new_string);
        write_file_after_edit(&app_handle, &validated_path, new_content, &config_read_guard).await?;
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
        write_file_after_edit(&app_handle, &validated_path, new_content, &config_read_guard).await?;
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

    let (best_match_value, similarity) = find_best_fuzzy_match_internal(&original_content, &normalized_old_string);
    let fuzzy_execution_time_ms = fuzzy_start_time.elapsed().as_secs_f64() * 1000.0;

    let diff_highlight = highlight_differences_internal(&normalized_old_string, &best_match_value);
    let char_code_data = get_character_code_data_internal(&normalized_old_string, &best_match_value);

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

    let logger_clone = fuzzy_logger_state.inner().clone(); // Get Arc<FuzzySearchLogger>
    tokio::spawn(async move {
        logger_clone.log(&log_entry).await;
    });

    let fuzzy_details = FuzzyMatchDetails {
        similarity_percent: similarity * 100.0,
        execution_time_ms: fuzzy_execution_time_ms,
        diff_highlight: diff_highlight.clone(),
        log_path_suggestion: config_read_guard.fuzzy_search_log_file.display().to_string(),
    };
    // drop(config_read_guard); // No longer needed

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

// Internal fuzzy matching logic (copied from original mcp-rg-editor's edit_tool.rs)
fn find_best_fuzzy_match_internal(text: &str, query: &str) -> (String, f64) {
    if text.is_empty() || query.is_empty() {
        return ("".to_string(), 0.0);
    }
    let mut best_similarity = 0.0;
    let mut best_match_str = "";
    let text_chars: Vec<char> = text.chars().collect();
    let text_len = text_chars.len();
    let query_len = query.chars().count();
    if query_len == 0 { return ("".to_string(), 0.0); }

    let min_window_len = std::cmp::max(1, query_len.saturating_sub(query_len / 4));
    let max_window_len = std::cmp::min(text_len, query_len + query_len / 4);

    for window_len_chars in min_window_len..=max_window_len {
        if window_len_chars > text_len { continue; }
        for i in 0..=(text_len - window_len_chars) {
            let start_byte_idx = text.char_indices().nth(i).map(|(idx, _)| idx).unwrap_or(0);
            let end_byte_idx = text.char_indices().nth(i + window_len_chars).map(|(idx, _)| idx).unwrap_or_else(|| text.len());
            let window_str_slice = &text[start_byte_idx..end_byte_idx];
            let current_similarity = strsim::jaro_winkler(window_str_slice, query);
            if current_similarity > best_similarity {
                best_similarity = current_similarity;
                best_match_str = window_str_slice;
            }
            if best_similarity > 0.999 {
                return (best_match_str.to_string(), best_similarity);
            }
        }
    }
    (best_match_str.to_string(), best_similarity)
}

fn highlight_differences_internal(expected: &str, actual: &str) -> String {
    let diff_results = diff::chars(expected, actual);
    let mut result = String::new();
    for d_res in diff_results {
        match d_res {
            diff::Result::Left(l) => result.push_str(&format!("{{-{}-}}", l)),
            diff::Result::Both(l, _) => result.push(l),
            diff::Result::Right(r) => result.push_str(&format!("{{+{}+}}", r)),
        }
    }
    result
}

struct CharCodeDataInternal {
    report: String,
    unique_count: usize,
    diff_length: usize,
}

fn get_character_code_data_internal(expected: &str, actual: &str) -> CharCodeDataInternal {
    use std::collections::HashMap;
    let mut prefix_len = 0;
    let min_char_len = std::cmp::min(expected.chars().count(), actual.chars().count());
    let mut expected_chars = expected.chars();
    let mut actual_chars = actual.chars();
    for _ in 0..min_char_len { if expected_chars.next() == actual_chars.next() { prefix_len +=1; } else { break; }}
    expected_chars = expected.chars().rev();
    actual_chars = actual.chars().rev();
    let mut suffix_len = 0;
    for _ in 0..(min_char_len - prefix_len) { if expected_chars.next() == actual_chars.next() { suffix_len +=1; } else { break; }}
    let expected_diff_str: String = expected.chars().skip(prefix_len).take(expected.chars().count() - prefix_len - suffix_len).collect();
    let actual_diff_str: String = actual.chars().skip(prefix_len).take(actual.chars().count() - prefix_len - suffix_len).collect();
    let mut char_codes: HashMap<u32, usize> = HashMap::new();
    let full_diff_str = format!("{}{}", expected_diff_str, actual_diff_str);
    for ch in full_diff_str.chars() { *char_codes.entry(ch as u32).or_insert(0) += 1; }
    let mut report_parts: Vec<String> = char_codes.iter().map(|(&code, &count)| {
        let char_display = std::char::from_u32(code)
            .map(|c| if c.is_control() || (c.is_whitespace() && c != ' ') { format!("\\x{:02x}", code) } else { c.to_string() })
            .unwrap_or_else(|| format!("\\u{{{:x}}}", code));
        format!("{}:{}[{}]", code, count, char_display)
    }).collect();
    report_parts.sort();
    CharCodeDataInternal { report: report_parts.join(","), unique_count: char_codes.len(), diff_length: full_diff_str.chars().count() }
}