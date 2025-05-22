use crate::config::Config;
use crate::error::AppError;
use crate::mcp::handler::ToolDependencies;
use crate::utils::fuzzy_search_logger::FuzzySearchLogEntry;
use crate::utils::line_ending_handler::{detect_line_ending, normalize_line_endings, LineEndingStyle};
use crate::utils::path_utils::validate_and_normalize_path;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
// use std::sync::RwLockReadGuard; // No longer needed directly in this file's functions
// use tauri::AppHandle; // No longer needed directly
use tauri_plugin_fs::FsExt; // Added FsExt
use tracing::{debug, instrument, warn}; // Corrected warn import
use std::time::Instant;
use chrono::Utc;
use diff;

// --- MCP Specific Parameter Struct ---
#[derive(Debug, Deserialize, Serialize)]
pub struct EditBlockParamsMCP {
    pub file_path: String,
    pub old_string: String,
    pub new_string: String,
    #[serde(default = "default_one_usize_mcp")]
    pub expected_replacements: usize,
}
fn default_one_usize_mcp() -> usize { 1 }

// --- MCP Specific Result Structs ---
#[derive(Debug, Serialize)]
pub struct EditBlockResultMCP {
    pub file_path: String,
    pub replacements_made: usize,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fuzzy_match_details: Option<FuzzyMatchDetailsMCP>,
}
#[derive(Debug, Serialize)]
pub struct FuzzyMatchDetailsMCP {
    pub similarity_percent: f64,
    pub execution_time_ms: f64,
    pub diff_highlight: String,
    pub log_path_suggestion: String,
}

const FUZZY_SIMILARITY_THRESHOLD_MCP: f64 = 0.7;

async fn read_file_for_edit_mcp_internal(
    app_handle: &tauri::AppHandle,
    file_path_str: &str,
    config: &Config // Pass &Config
) -> Result<(String, PathBuf, LineEndingStyle), AppError> {
    // validate_and_normalize_path now takes &Config
    let path = validate_and_normalize_path(file_path_str, config, true, false)?;
    if !app_handle.fs_scope().is_allowed(&path) {
        return Err(AppError::PathNotAllowed(format!("Read denied by FS scope: {}", path.display())));
    }
    // Use app_handle.fs() for filesystem operations
    let original_content = app_handle.fs().read_text_file(&path).await
        .map_err(|e| AppError::PluginError{plugin:"fs".to_string(), message:format!("Failed to read text file {}: {}", path.display(), e)})?;
    Ok((original_content, path, detect_line_ending(&original_content)))
}


async fn write_file_after_edit_mcp(
    app_handle: &tauri::AppHandle,
    path_obj: &PathBuf,
    content: String
) -> Result<(), AppError> {
    if !app_handle.fs_scope().is_allowed(path_obj) {
        return Err(AppError::PathNotAllowed(format!("Write denied by FS scope: {}", path_obj.display())));
    }
    // Use app_handle.fs() for filesystem operations
    app_handle.fs().write_text_file(path_obj, content).await
        .map_err(|e| AppError::PluginError{plugin:"fs".to_string(), message:format!("Failed to write text file {}: {}", path_obj.display(), e)})
}


#[instrument(skip(deps, params), fields(file_path = %params.file_path))]
pub async fn mcp_edit_block(
    deps: &ToolDependencies,
    params: EditBlockParamsMCP
) -> Result<EditBlockResultMCP, AppError> {
    if params.old_string.is_empty() { return Err(AppError::EditError("old_string cannot be empty.".into())); }

    let (original_content, validated_path, file_line_ending, fuzzy_log_path, _files_root_for_log) = { // _files_root_for_log marked unused
        let config_guard = deps.config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock: {}", e)))?;
        // Pass &*config_guard to get &Config
        let (content, path, ending) = read_file_for_edit_mcp_internal(&deps.app_handle, &params.file_path, &*config_guard).await?;
        (content, path, ending, config_guard.fuzzy_search_log_file.clone(), config_guard.files_root.clone())
    };
    
    let file_ext = validated_path.extension().unwrap_or_default().to_string_lossy().to_lowercase();

    let norm_old = normalize_line_endings(&params.old_string, file_line_ending);
    let norm_new = normalize_line_endings(&params.new_string, file_line_ending);
    let occurrences: Vec<_> = original_content.match_indices(&norm_old).collect();
    let actual_occurrences = occurrences.len();

    if (params.expected_replacements > 0 && actual_occurrences == params.expected_replacements) ||
       (params.expected_replacements == 0 && actual_occurrences > 0) {
        let new_content = original_content.replace(&norm_old, &norm_new);
        write_file_after_edit_mcp(&deps.app_handle, &validated_path, new_content).await?;
        let msg_key = if params.expected_replacements == 0 {"all occurrences"} else {"exact replacement(s)"};
        return Ok(EditBlockResultMCP {
            file_path: params.file_path,
            replacements_made: actual_occurrences,
            message: format!("Successfully applied {} {}.", actual_occurrences, msg_key),
            fuzzy_match_details: None
        });
    }

    if actual_occurrences > 0 && params.expected_replacements > 0 && actual_occurrences != params.expected_replacements {
         return Err(AppError::EditError(format!(
            "Expected {} occurrences but found {}. Please verify 'old_string' for uniqueness or adjust 'expected_replacements'. To replace all {} occurrences, set expected_replacements to 0 or {}.",
            params.expected_replacements, actual_occurrences, actual_occurrences, actual_occurrences
        )));
    }

    debug!("No exact match or count mismatch. Attempting fuzzy search for MCP edit_block.");
    let fuzzy_start = Instant::now();
    let (best_match, similarity) = find_best_fuzzy_match_internal(&original_content, &norm_old);
    let fuzzy_time_ms = fuzzy_start.elapsed().as_secs_f64() * 1000.0;
    let diff_hl = highlight_differences_internal(&norm_old, &best_match);
    let char_data = get_character_code_data_internal(&norm_old, &best_match);

    let log_entry = FuzzySearchLogEntry {
        timestamp: Utc::now(), search_text: params.old_string.clone(), found_text: best_match.clone(), similarity,
        execution_time_ms: fuzzy_time_ms, exact_match_count: actual_occurrences, expected_replacements: params.expected_replacements,
        fuzzy_threshold: FUZZY_SIMILARITY_THRESHOLD_MCP, below_threshold: similarity < FUZZY_SIMILARITY_THRESHOLD_MCP,
        diff: diff_hl.clone(), search_length: params.old_string.len(), found_length: best_match.len(),
        file_extension: file_ext.to_string(), character_codes: char_data.report,
        unique_character_count: char_data.unique_count, diff_length: char_data.diff_length,
    };
    let logger_clone = deps.fuzzy_search_logger.clone();
    tokio::spawn(async move { logger_clone.log(&log_entry).await; });

    let fuzzy_details = FuzzyMatchDetailsMCP {
        similarity_percent: similarity * 100.0, execution_time_ms: fuzzy_time_ms,
        diff_highlight: diff_hl.clone(), log_path_suggestion: fuzzy_log_path.display().to_string()
    };
    
    if similarity >= FUZZY_SIMILARITY_THRESHOLD_MCP {
        Ok(EditBlockResultMCP {
            file_path: params.file_path, replacements_made: 0,
            message: format!("Exact match not found. Similar text found ({:.2}% similarity). Review diff and provide exact text if replacement desired.", similarity * 100.0),
            fuzzy_match_details: Some(fuzzy_details)
        })
    } else {
        Err(AppError::EditError(format!("Search string not found. Closest fuzzy match {:.2}% (threshold {}%). Diff: {}", similarity * 100.0, FUZZY_SIMILARITY_THRESHOLD_MCP * 100.0, diff_hl)))
    }
}

fn find_best_fuzzy_match_internal(text: &str, query: &str) -> (String, f64) {
    if text.is_empty() || query.is_empty() { return ("".to_string(), 0.0); }
    let mut best_similarity = 0.0; let mut best_match_str = "";
    let text_chars: Vec<char> = text.chars().collect(); let text_len = text_chars.len();
    let query_len = query.chars().count(); if query_len == 0 { return ("".to_string(), 0.0); }
    let min_window_len = std::cmp::max(1, query_len.saturating_sub(query_len / 4));
    let max_window_len = std::cmp::min(text_len, query_len + query_len / 4);
    for window_len_chars in min_window_len..=max_window_len { if window_len_chars > text_len { continue; }
        for i in 0..=(text_len - window_len_chars) {
            let start_byte_idx = text.char_indices().nth(i).map(|(idx, _)| idx).unwrap_or(0);
            let end_byte_idx = text.char_indices().nth(i + window_len_chars).map(|(idx, _)| idx).unwrap_or_else(|| text.len());
            let window_str_slice = &text[start_byte_idx..end_byte_idx];
            let current_similarity = strsim::jaro_winkler(window_str_slice, query);
            if current_similarity > best_similarity { best_similarity = current_similarity; best_match_str = window_str_slice; }
            if best_similarity > 0.999 { return (best_match_str.to_string(), best_similarity); }
        }
    } (best_match_str.to_string(), best_similarity)
}
fn highlight_differences_internal(expected: &str, actual: &str) -> String {
    let diff_results = diff::chars(expected, actual); let mut result = String::new();
    for d_res in diff_results { match d_res {
        diff::Result::Left(l) => result.push_str(&format!("{{-{}-}}", l)),
        diff::Result::Both(l, _) => result.push(l),
        diff::Result::Right(r) => result.push_str(&format!("{{+{}+}}", r)),
    }} result
}
struct CharCodeDataInternal { report: String, unique_count: usize, diff_length: usize }
fn get_character_code_data_internal(expected: &str, actual: &str) -> CharCodeDataInternal {
    use std::collections::HashMap; let mut prefix_len = 0;
    let min_char_len = std::cmp::min(expected.chars().count(), actual.chars().count());
    let mut expected_chars_iter_prefix = expected.chars();
    let mut actual_chars_iter_prefix = actual.chars();
    for _ in 0..min_char_len { if expected_chars_iter_prefix.next() == actual_chars_iter_prefix.next() { prefix_len +=1; } else { break; }}
    
    let mut expected_chars_rev_iter = expected.chars().rev();
    let mut actual_chars_rev_iter = actual.chars().rev();
    let mut suffix_len = 0;
    for _ in 0..(min_char_len - prefix_len) { if expected_chars_rev_iter.next() == actual_chars_rev_iter.next() { suffix_len +=1; } else { break; }}
    
    let expected_diff_str: String = expected.chars().skip(prefix_len).take(expected.chars().count().saturating_sub(prefix_len).saturating_sub(suffix_len)).collect();
    let actual_diff_str: String = actual.chars().skip(prefix_len).take(actual.chars().count().saturating_sub(prefix_len).saturating_sub(suffix_len)).collect();
    
    let mut char_codes: HashMap<u32, usize> = HashMap::new();
    let full_diff_str = format!("{}{}", expected_diff_str, actual_diff_str);
    for ch in full_diff_str.chars() { *char_codes.entry(ch as u32).or_insert(0) += 1; }
    let mut report_parts: Vec<String> = char_codes.iter().map(|(&code, &count)| {
        let char_display = std::char::from_u32(code).map(|c| if c.is_control() || (c.is_whitespace() && c != ' ') { format!("\\x{:02x}", code) } else { c.to_string() }).unwrap_or_else(|| format!("\\u{{{:x}}}", code));
        format!("{}:{}[{}]", code, count, char_display)
    }).collect(); report_parts.sort();
    CharCodeDataInternal { report: report_parts.join(","), unique_count: char_codes.len(), diff_length: full_diff_str.chars().count() }
}