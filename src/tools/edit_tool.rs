use crate::config::Config;
use crate::error::AppError;
use crate::tools::filesystem_tool::{FilesystemManager, ReadFileParams, WriteFileParams, WriteMode};
use crate::utils::fuzzy_search_logger::{FuzzySearchLogger, FuzzySearchLogEntry};
use crate::utils::line_ending_handler::{detect_line_ending, normalize_line_endings, LineEndingStyle};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, Mutex}; // Mutex for FuzzySearchLogger
use tracing::{debug, instrument, warn};
use std::time::Instant;

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
    config: Arc<Config>,
    filesystem_manager: Arc<FilesystemManager>,
    fuzzy_logger: Arc<Mutex<FuzzySearchLogger>>,
}

impl EditManager {
    pub fn new(config: Arc<Config>, filesystem_manager: Arc<FilesystemManager>) -> Self {
        let fuzzy_logger = Arc::new(Mutex::new(FuzzySearchLogger::new(config.clone())));
        Self { config, filesystem_manager, fuzzy_logger }
    }

    #[instrument(skip(self, params), fields(file_path = %params.file_path))]
    pub async fn edit_block(&self, params: &EditBlockParams) -> Result<EditBlockResult, AppError> {
        debug!("Editing block in file: {}", params.file_path);

        if params.old_string.is_empty() {
            return Err(AppError::EditError("Search string (old_string) cannot be empty.".to_string()));
        }

        let file_path_obj = Path::new(¶ms.file_path);
        let file_extension = file_path_obj.extension().unwrap_or_default().to_string_lossy().to_lowercase();

        // Read entire file for editing. We'll handle large files by advising user to make smaller edits.
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

        let normalized_old_string = normalize_line_endings(¶ms.old_string, file_line_ending);
        let normalized_new_string = normalize_line_endings(¶ms.new_string, file_line_ending);

        // --- Exact Match Logic ---
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

        if actual_occurrences > 0 && params.expected_replacements > 0 && actual_occurrences != params.expected_replacements {
             return Err(AppError::EditError(format!(
                "Expected {} occurrences but found {}. Please verify 'old_string' for uniqueness or adjust 'expected_replacements'. If you want to replace all {} occurrences, set expected_replacements to {}.",
                params.expected_replacements, actual_occurrences, actual_occurrences, actual_occurrences
            )));
        }
        
        // If expected_replacements is 0, it means replace all occurrences if any are found.
        // This case is now handled by the above block if actual_occurrences > 0.
        // If actual_occurrences is 0, we proceed to fuzzy search.

        // --- Fuzzy Match Logic (if no exact match or mismatch in expected count for specific replacement) ---
        debug!("No exact match or count mismatch. Attempting fuzzy search.");
        let fuzzy_start_time = Instant::now();
        
        // For simplicity, let's find the single best fuzzy match in the entire content.
        // A more advanced approach might iterate or find multiple fuzzy matches.
        let (best_match_value, similarity) = find_best_fuzzy_match(&original_content, &normalized_old_string);
        let fuzzy_execution_time_ms = fuzzy_start_time.elapsed().as_secs_f64() * 1000.0;

        let diff_highlight = highlight_differences(&normalized_old_string, &best_match_value);
        let char_code_data = get_character_code_data(&normalized_old_string, &best_match_value);

        let log_entry = FuzzySearchLogEntry {
            timestamp: Utc::now(),
            search_text: params.old_string.clone(), // Log original user input
            found_text: best_match_value.clone(),
            similarity,
            execution_time_ms: fuzzy_execution_time_ms,
            exact_match_count: actual_occurrences, // Could be 0 if no exact match at all
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
        
        // Log asynchronously
        let logger = self.fuzzy_logger.clone();
        tokio::spawn(async move {
            let mut guard = logger.lock().expect("Failed to lock fuzzy logger");
            guard.log(&log_entry).await;
        });

        let fuzzy_details = FuzzyMatchDetails {
            similarity_percent: similarity * 100.0,
            execution_time_ms: fuzzy_execution_time_ms,
            diff_highlight: diff_highlight.clone(),
            log_path_suggestion: self.config.fuzzy_search_log_file.display().to_string(),
        };

        if similarity >= FUZZY_SIMILARITY_THRESHOLD {
            warn!(similarity, "Fuzzy match found, but not applied automatically.");
            // Do NOT replace on fuzzy match automatically for safety.
            // Claude should be informed to refine the `old_string` based on the diff.
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
    let mut best_match_str = "";

    // Sliding window approach
    let text_len = text.chars().count();
    let query_len = query.chars().count();
    
    // Consider windows slightly larger and smaller than the query length
    let min_window = std::cmp::max(1, query_len / 2);
    let max_window = std::cmp::min(text_len, query_len + query_len / 2);

    for len_diff in -(query_len as isize / 4)..(query_len as isize / 4 + 1) {
        let window_len = (query_len as isize + len_diff) as usize;
        if window_len == 0 || window_len > text_len { continue; }

        for i in 0..=(text_len - window_len) {
            let window_str: String = text.chars().skip(i).take(window_len).collect();
            let current_similarity = strsim::jaro_winkler(&window_str, query);
            if current_similarity > best_similarity {
                best_similarity = current_similarity;
                best_match_str = &text[text.char_indices().nth(i).unwrap().0 .. text.char_indices().nth(i + window_len).map_or(text.len(), |(idx, _)| idx)];
            }
            if best_similarity > 0.99 { // Early exit for near perfect match
                return (best_match_str.to_string(), best_similarity);
            }
        }
    }
    (best_match_str.to_string(), best_similarity)
}


fn highlight_differences(expected: &str, actual: &str) -> String {
    let diff = diff::chars(expected, actual);
    let mut result = String::new();
    for d in diff {
        match d {
            diff::Result::Left(l) => result.push_str(&format!("{{-{}-}}", l)),
            diff::Result::Both(l, _) => result.push_str(l),
            diff::Result::Right(r) => result.push_str(&format!("{{+{}+}}", r)),
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
    let min_len = std::cmp::min(expected.len(), actual.len());
    let expected_bytes = expected.as_bytes();
    let actual_bytes = actual.as_bytes();

    while prefix_len < min_len && expected_bytes[prefix_len] == actual_bytes[prefix_len] {
        prefix_len += 1;
    }

    let mut suffix_len = 0;
    while suffix_len < min_len - prefix_len
        && expected_bytes[expected.len() - 1 - suffix_len] == actual_bytes[actual.len() - 1 - suffix_len]
    {
        suffix_len += 1;
    }

    let expected_diff_str = &expected[prefix_len..expected.len() - suffix_len];
    let actual_diff_str = &actual[prefix_len..actual.len() - suffix_len];

    let mut char_codes: HashMap<u32, usize> = HashMap::new();
    let full_diff_str = format!("{}{}", expected_diff_str, actual_diff_str);

    for ch in full_diff_str.chars() {
        *char_codes.entry(ch as u32).or_insert(0) += 1;
    }
    
    let mut report_parts: Vec<String> = char_codes
        .iter()
        .map(|(&code, &count)| {
            let char_display = std::char::from_u32(code)
                .map(|c| if c.is_control() || c.is_whitespace() { format!("\\x{:02x}", code) } else { c.to_string() })
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