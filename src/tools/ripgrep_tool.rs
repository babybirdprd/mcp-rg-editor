use crate::config::Config;
use crate::error::AppError;
use crate::utils::path_utils::validate_path_access;
use serde::{Deserialize, Serialize};
use std::path::PathBuf; // Removed Path as not directly used
use std::process::Stdio as StdProcessStdio;
use std::sync::{Arc, RwLock as StdRwLock}; // Changed to StdRwLock for Config
use tokio::process::Command as TokioCommand;
use tokio::time::{timeout, Duration};
use tracing::{debug, error, instrument, warn};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SearchCodeParams {
    pub pattern: String,
    #[serde(default)]
    pub path: String,
    #[serde(default, alias = "fixedStrings")]
    pub fixed_strings: bool,
    #[serde(default, alias = "ignoreCase")]
    pub ignore_case: bool,
    #[serde(default)] // Added default for case_sensitive
    pub case_sensitive: bool, // Added this field
    #[serde(default = "default_true", alias = "lineNumbers")]
    pub line_numbers: bool,
    #[serde(alias = "contextLines")]
    pub context_lines: Option<usize>,
    #[serde(default, alias = "filePattern")]
    pub file_pattern: Option<String>,
    #[serde(alias = "maxDepth")]
    pub max_depth: Option<usize>,
    #[serde(default = "default_usize_1000", alias = "maxResults")]
    pub max_results: usize,
    #[serde(default, alias = "includeHidden")]
    pub include_hidden: bool,
    #[serde(default, rename = "timeoutMs")]
    pub timeout_ms: Option<u64>,
}

fn default_true() -> bool { true }
fn default_usize_1000() -> usize { 1000 }


#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RipgrepMatch {
    pub file: String,
    pub line: u64,
    pub match_text: String,
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
    pub elapsed_ms: u64,
}

#[derive(Debug)] // Added Debug
pub struct RipgrepSearcher {
    config: Arc<StdRwLock<Config>>, // Changed to StdRwLock
    rg_path: PathBuf,
}

impl RipgrepSearcher {
    pub fn new(config: Arc<StdRwLock<Config>>) -> Self { // Changed to StdRwLock
        let rg_path = which::which("rg").unwrap_or_else(|_| {
            warn!("ripgrep (rg) not found in PATH. search_code tool will not function.");
            PathBuf::from("rg")
        });
        Self { config, rg_path }
    }

    #[instrument(skip(self, params), fields(pattern = %params.pattern, path = %params.path))]
    pub async fn search_code(&self, params: &SearchCodeParams) -> Result<SearchCodeResult, AppError> {
        if !self.rg_path.exists() && which::which(&self.rg_path).is_err() {
             return Err(AppError::RipgrepError("ripgrep (rg) executable not found in PATH. Please install ripgrep.".to_string()));
        }
        debug!("Starting ripgrep search_code with params: {:?}", params);
        
        let config_guard = self.config.read().map_err(|_| AppError::ConfigError(anyhow::anyhow!("Config lock poisoned")))?;


        let search_dir_str = if params.path.is_empty() {
            config_guard.files_root.to_str().unwrap_or(".").to_string()
        } else {
            params.path.clone()
        };
        
        let search_path_validated = validate_path_access(&search_dir_str, &config_guard, true)?;
        
        let start_time = std::time::Instant::now();
        let mut cmd = TokioCommand::new(&self.rg_path);

        cmd.current_dir(&config_guard.files_root);

        cmd.arg("--json"); 
        if params.line_numbers {
            cmd.arg("--line-number");
        }
        if params.fixed_strings {
            cmd.arg("-F");
        }
        
        // Ripgrep logic: -s (case-sensitive) overrides -i (ignore-case). Smart case is default.
        if params.case_sensitive {
            cmd.arg("-s");
        } else if params.ignore_case { // Only apply ignore_case if case_sensitive is false
            cmd.arg("-i");
        }


        if let Some(context) = params.context_lines {
            cmd.arg("-C").arg(context.to_string());
        }
        if let Some(glob) = &params.file_pattern { // Corrected: &params.file_pattern
            cmd.arg("-g").arg(glob);
        }
        if let Some(depth) = params.max_depth {
            cmd.arg("--max-depth").arg(depth.to_string());
        }
        cmd.arg("--max-count").arg(params.max_results.to_string());
        if params.include_hidden {
            cmd.arg("--hidden");
        }
        
        cmd.arg(&params.pattern); // Corrected: params.pattern
        cmd.arg(&search_path_validated);

        cmd.stdout(StdProcessStdio::piped());
        cmd.stderr(StdProcessStdio::piped());
        
        let files_root_clone_for_path_processing = config_guard.files_root.clone(); // Clone for use in parsing
        drop(config_guard); // Release lock

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
        
        let timeout_duration = Duration::from_millis(params.timeout_ms.unwrap_or(30000));
        
        match timeout(timeout_duration, child_process_future).await {
            Ok(Ok(output)) => {
                let elapsed_ms = start_time.elapsed().as_millis() as u64;

                if !output.status.success() && output.status.code() != Some(1) {
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
                                        full_match_line.push_str(lines_text_val.as_str().unwrap_or(""));
                                    }

                                    let absolute_match_path = files_root_clone_for_path_processing.join(path_text);
                                    let display_path = match absolute_match_path.strip_prefix(&files_root_clone_for_path_processing) {
                                        Ok(p) => p.to_string_lossy().into_owned(),
                                        Err(_) => path_text.to_string(),
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

                                    let absolute_match_path = files_root_clone_for_path_processing.join(path_text);
                                    let display_path = match absolute_match_path.strip_prefix(&files_root_clone_for_path_processing) {
                                        Ok(p) => p.to_string_lossy().into_owned(),
                                        Err(_) => path_text.to_string(),
                                    };
                                    matches.push(RipgrepMatch {
                                        file: display_path,
                                        line: line_num,
                                        match_text: context_line_text.trim_end().to_string(),
                                    });
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
            Ok(Err(app_error)) => Err(app_error),
            Err(_) => {
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