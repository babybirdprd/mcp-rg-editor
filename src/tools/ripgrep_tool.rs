use crate::config::Config;
use crate::error::AppError;
use crate::utils::path_utils::validate_path_access;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::Stdio as StdProcessStdio; // Alias to avoid conflict with tokio::process::Stdio
use std::sync::Arc;
use tokio::process::Command as TokioCommand;
use tokio::time::{timeout, Duration};
use tracing::{debug, error, instrument, warn};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SearchCodeParams {
    pub pattern: String,
    #[serde(default)]
    pub path: String, // Path to search in, can be relative to FILES_ROOT or absolute if allowed
    #[serde(default, alias = "fixedStrings")]
    pub fixed_strings: bool,
    #[serde(default, alias = "ignoreCase")] // Match desktop-commander schema
    pub ignore_case: bool, // rg default is smart case, -i for ignore, -s for case-sensitive
    #[serde(default = "default_true", alias = "lineNumbers")]
    pub line_numbers: bool,
    #[serde(alias = "contextLines")]
    pub context_lines: Option<usize>,
    #[serde(default, alias = "filePattern")] // Match desktop-commander schema
    pub file_pattern: Option<String>, // rg -g/--glob
    #[serde(alias = "maxDepth")]
    pub max_depth: Option<usize>,
    #[serde(default = "default_usize_1000", alias = "maxResults")]
    pub max_results: usize,
    #[serde(default, alias = "includeHidden")] // Match desktop-commander schema
    pub include_hidden: bool,
    #[serde(default, rename = "timeoutMs")]
    pub timeout_ms: Option<u64>,
}

fn default_true() -> bool { true }
fn default_usize_1000() -> usize { 1000 }


#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RipgrepMatch {
    pub file: String, // Relative to search root
    pub line: u64,
    pub match_text: String, // The actual matched line content
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SearchCodeResult {
    pub matches: Vec<RipgrepMatch>,
    pub stats: SearchStats,
    pub timed_out: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SearchStats {
    pub matched_lines: usize,
    // Ripgrep JSON output doesn't easily provide total files searched or bytes.
    // We could add a summary run with --stats if needed, but it adds overhead.
    pub elapsed_ms: u64,
}

#[derive(Debug)]
pub struct RipgrepSearcher {
    config: Arc<Config>,
    rg_path: PathBuf,
}

impl RipgrepSearcher {
    pub fn new(config: Arc<Config>) -> Self {
        let rg_path = which::which("rg").unwrap_or_else(|_| {
            warn!("ripgrep (rg) not found in PATH. search_code tool will not function.");
            PathBuf::from("rg") // Store it anyway, execution will fail if not found
        });
        Self { config, rg_path }
    }

    #[instrument(skip(self, params), fields(pattern = %params.pattern, path = %params.path))]
    pub async fn search_code(&self, params: &SearchCodeParams) -> Result<SearchCodeResult, AppError> {
        if !self.rg_path.exists() && which::which(&self.rg_path).is_err() {
             return Err(AppError::RipgrepError("ripgrep (rg) executable not found in PATH. Please install ripgrep.".to_string()));
        }
        debug!("Starting ripgrep search_code with params: {:?}", params);

        let search_dir_str = if params.path.is_empty() {
            self.config.files_root.to_str().unwrap_or(".").to_string()
        } else {
            params.path.clone()
        };
        
        let search_path_validated = validate_path_access(&search_dir_str, &self.config, true)?;
        
        let start_time = std::time::Instant::now();
        let mut cmd = TokioCommand::new(&self.rg_path);

        cmd.current_dir(&self.config.files_root); // Run rg from files_root for consistent relative paths

        cmd.arg("--json"); 
        if params.line_numbers {
            cmd.arg("--line-number");
        }
        if params.fixed_strings {
            cmd.arg("-F");
        }
        if params.ignore_case {
            cmd.arg("-i");
        } else if params.case_sensitive { // Explicit case sensitive if ignore_case is false
            cmd.arg("-s");
        }
        // Smart case is rg's default if neither -i nor -s is given.

        if let Some(context) = params.context_lines {
            cmd.arg("-C").arg(context.to_string());
        }
        if let Some(glob) = ¶ms.file_pattern {
            cmd.arg("-g").arg(glob);
        }
        if let Some(depth) = params.max_depth {
            cmd.arg("--max-depth").arg(depth.to_string());
        }
        cmd.arg("--max-count").arg(params.max_results.to_string());
        if params.include_hidden {
            cmd.arg("--hidden");
        }
        
        cmd.arg(¶ms.pattern);
        cmd.arg(&search_path_validated); // Use the validated, absolute path

        cmd.stdout(StdProcessStdio::piped());
        cmd.stderr(StdProcessStdio::piped());

        debug!("Executing rg command: {:?}", cmd);
        
        let child_process_future = async {
            let mut child = cmd.spawn().map_err(|e| {
                error!("Failed to spawn ripgrep: {}", e);
                AppError::RipgrepError(format!("Failed to spawn ripgrep: {}", e))
            })?;

            let output = child.wait_with_output().await.map_err(|e| {
                error!("Failed to wait for ripgrep output: {}", e);
                AppError::RipgrepError(format!("Failed to read ripgrep output: {}", e))
            })?;
            Ok(output)
        };
        
        let timeout_duration = Duration::from_millis(params.timeout_ms.unwrap_or(30000)); // Default 30s
        
        match timeout(timeout_duration, child_process_future).await {
            Ok(Ok(output)) => { // Command finished within timeout
                let elapsed_ms = start_time.elapsed().as_millis() as u64;

                if !output.status.success() && output.status.code() != Some(1) { // code 1 means no matches found, which is not an error for us
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

                for line_str in stdout.lines() {
                    if line_str.trim().is_empty() { continue; }
                    match serde_json::from_str::<serde_json::Value>(line_str) {
                        Ok(json_val) => {
                            if json_val.get("type").and_then(|t| t.as_str()) == Some("match") {
                                if let Some(data) = json_val.get("data") {
                                    let path_text = data.get("path").and_then(|p| p.get("text")).and_then(|t| t.as_str()).unwrap_or_default();
                                    let line_num = data.get("line_number").and_then(|n| n.as_u64()).unwrap_or(0);
                                    
                                    let mut full_match_line = String::new();
                                    if let Some(submatches_array) = data.get("submatches").and_then(|s| s.as_array()) {
                                        for submatch_obj in submatches_array {
                                            if let Some(match_text_val) = submatch_obj.get("match").and_then(|m| m.get("text")) {
                                                full_match_line.push_str(match_text_val.as_str().unwrap_or(""));
                                            }
                                        }
                                    } else if let Some(lines_text_val) = data.get("lines").and_then(|l| l.get("text")) {
                                        // This handles context lines if submatches are not present for the main match line
                                        full_match_line.push_str(lines_text_val.as_str().unwrap_or(""));
                                    }


                                    // Make path relative to files_root for consistent output
                                    let absolute_match_path = self.config.files_root.join(path_text);
                                    let display_path = match absolute_match_path.strip_prefix(&self.config.files_root) {
                                        Ok(p) => p.to_string_lossy().into_owned(),
                                        Err(_) => path_text.to_string(), // fallback if stripping fails
                                    };

                                    matches.push(RipgrepMatch {
                                        file: display_path,
                                        line: line_num,
                                        match_text: full_match_line.trim_end().to_string(),
                                    });
                                    matched_lines_count += 1;
                                }
                            } else if json_val.get("type").and_then(|t| t.as_str()) == Some("context") && params.context_lines.unwrap_or(0) > 0 {
                                 if let Some(data) = json_val.get("data") {
                                    let path_text = data.get("path").and_then(|p| p.get("text")).and_then(|t| t.as_str()).unwrap_or_default();
                                    let line_num = data.get("line_number").and_then(|n| n.as_u64()).unwrap_or(0);
                                    let context_line_text = data.get("lines").and_then(|l| l.get("text")).and_then(|t| t.as_str()).unwrap_or_default();

                                    let absolute_match_path = self.config.files_root.join(path_text);
                                    let display_path = match absolute_match_path.strip_prefix(&self.config.files_root) {
                                        Ok(p) => p.to_string_lossy().into_owned(),
                                        Err(_) => path_text.to_string(),
                                    };
                                    matches.push(RipgrepMatch {
                                        file: display_path,
                                        line: line_num,
                                        match_text: context_line_text.trim_end().to_string(),
                                    });
                                    // Don't increment matched_lines_count for context lines
                                 }
                            }
                        }
                        Err(e) => {
                            warn!(error = %e, line = %line_str, "Failed to parse ripgrep JSON line");
                        }
                    }
                }
                
                Ok(SearchCodeResult {
                    matches,
                    stats: SearchStats {
                        matched_lines: matched_lines_count,
                        elapsed_ms,
                    },
                    timed_out: false,
                })
            },
            Ok(Err(app_error)) => Err(app_error), // Error from spawning/running the command
            Err(_) => { // Timeout occurred
                let elapsed_ms = start_time.elapsed().as_millis() as u64;
                warn!(pattern = %params.pattern, path = %params.path, timeout = timeout_duration.as_millis(), "Ripgrep search timed out");
                Ok(SearchCodeResult {
                    matches: vec![],
                    stats: SearchStats {
                        matched_lines: 0,
                        elapsed_ms,
                    },
                    timed_out: true,
                })
            }
        }
    }
}