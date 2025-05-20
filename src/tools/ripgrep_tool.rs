use crate::config::Config;
use crate::error::AppError;
use crate::utils::path_utils::validate_path_access;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tokio::process::Command as TokioCommand;
use tracing::{debug, error, instrument};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SearchCodeParams {
    pub pattern: String,
    #[serde(default)]
    pub path: String, // Relative to FILES_ROOT
    #[serde(default)]
    pub fixed_strings: bool,
    #[serde(default = "default_true")]
    pub case_sensitive: bool, // ripgrep is case-insensitive by default, so true means pass -s
    #[serde(default = "default_true")]
    pub line_numbers: bool,
    pub context_lines: Option<usize>,
    #[serde(default)]
    pub file_types: Vec<String>,
    pub max_depth: Option<usize>,
    #[serde(default = "default_usize_1000")]
    pub max_results: usize,
}

fn default_true() -> bool { true }
fn default_usize_1000() -> usize { 1000 }


#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SearchCodeResult {
    pub matches: Vec<String>,
    pub stats: SearchStats,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SearchStats {
    pub matched_lines: usize,
    pub files_searched: Option<usize>, // Ripgrep doesn't easily provide this
    pub elapsed_ms: u64,
}

#[derive(Debug)]
pub struct RipgrepSearcher {
    config: Arc<Config>,
}

impl RipgrepSearcher {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }

    #[instrument(skip(self, params), fields(pattern = %params.pattern, path = %params.path))]
    pub async fn search_code(&self, params: &SearchCodeParams) -> Result<SearchCodeResult, AppError> {
        debug!("Starting ripgrep search_code");

        let search_dir_str = if params.path.is_empty() {
            self.config.files_root.to_str().unwrap_or(".").to_string()
        } else {
            params.path.clone()
        };
        
        let search_path = validate_path_access(&search_dir_str, &self.config, true)?;
        
        let start_time = std::time::Instant::now();
        let mut cmd = TokioCommand::new("rg");

        cmd.arg("--json"); // Output results as JSON lines for easier parsing
        cmd.current_dir(&self.config.files_root); // Run rg from files_root

        if params.fixed_strings {
            cmd.arg("--fixed-strings");
        }
        if params.case_sensitive {
            cmd.arg("--case-sensitive");
        } else {
            cmd.arg("--ignore-case");
        }
        if params.line_numbers {
            cmd.arg("--line-number");
        }
        if let Some(context) = params.context_lines {
            cmd.arg("--context").arg(context.to_string());
        }
        for ft in ¶ms.file_types {
            cmd.arg("--type").arg(ft);
        }
        if let Some(depth) = params.max_depth {
            cmd.arg("--max-depth").arg(depth.to_string());
        }
        cmd.arg("--max-count").arg(params.max_results.to_string());

        cmd.arg(¶ms.pattern);
        cmd.arg(&search_path); // search_path is now absolute and validated

        debug!("Executing rg command: {:?}", cmd);
        let output = cmd.output().await.map_err(|e| {
            error!("Failed to execute ripgrep: {}", e);
            AppError::RipgrepError(format!("Failed to execute ripgrep: {}", e))
        })?;

        let elapsed_ms = start_time.elapsed().as_millis() as u64;

        if !output.status.success() && output.status.code() != Some(1) { // code 1 means no matches
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!("Ripgrep command failed with status {:?}: {}", output.status, stderr);
            return Err(AppError::RipgrepError(format!(
                "Ripgrep failed (status: {:?}): {}",
                output.status, stderr
            )));
        }

        let stdout = String::from_utf8(output.stdout).map_err(|e| {
            error!("Ripgrep output is not valid UTF-8: {}", e);
            AppError::RipgrepError(format!("Ripgrep output not UTF-8: {}", e))
        })?;

        let mut matches = Vec::new();
        let mut matched_lines_count = 0;

        for line in stdout.lines() {
            if let Ok(json_val) = serde_json::from_str::<serde_json::Value>(line) {
                if json_val["type"] == "match" {
                    matched_lines_count +=1;
                    let path_text = json_val["data"]["path"]["text"].as_str().unwrap_or_default();
                    let line_num = json_val["data"]["line_number"].as_u64().unwrap_or_default();
                    
                    // Process submatches to reconstruct the full matching line
                    let mut full_match_line = String::new();
                    if let Some(submatches_array) = json_val["data"]["submatches"].as_array() {
                        for submatch_obj in submatches_array {
                             if let Some(match_text_val) = submatch_obj.get("match") {
                                if let Some(text_val) = match_text_val.get("text"){
                                     full_match_line.push_str(text_val.as_str().unwrap_or(""));
                                }
                             }
                        }
                    }
                    
                    // Make path relative to files_root for consistent output
                    let absolute_match_path = self.config.files_root.join(path_text);
                    let relative_match_path = match absolute_match_path.strip_prefix(&self.config.files_root) {
                        Ok(p) => p.to_path_buf(),
                        Err(_) => PathBuf::from(path_text), // fallback if stripping fails
                    };

                    matches.push(format!(
                        "{}:{}:{}",
                        relative_match_path.display(),
                        line_num,
                        full_match_line.trim_end()
                    ));
                }
            }
        }
        
        Ok(SearchCodeResult {
            matches,
            stats: SearchStats {
                matched_lines: matched_lines_count,
                files_searched: None, // Ripgrep JSON output doesn't easily give total files searched
                elapsed_ms,
            },
        })
    }
}