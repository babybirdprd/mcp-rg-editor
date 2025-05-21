// FILE: src-tauri/src/commands/ripgrep_commands.rs
// IMPORTANT NOTE: Rewrite the entire file.
use crate::config::Config;
use crate::error::AppError;
use crate::utils::path_utils::validate_and_normalize_path;
use crate::utils::audit_logger::audit_log;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{Arc, RwLock as StdRwLock};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_shell::ShellExt;
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
    #[serde(default)]
    pub case_sensitive: bool,
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
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SearchStats {
    pub matched_lines: usize,
    pub elapsed_ms: u64,
}

fn get_rg_path() -> Result<PathBuf, AppError> {
    which::which("rg").map_err(|e| {
        warn!("ripgrep (rg) not found in PATH. search_code tool will not function. Error: {}", e);
        AppError::RipgrepError("ripgrep (rg) executable not found in PATH. Please install ripgrep.".to_string())
    })
}

#[tauri::command(async)]
#[instrument(skip(app_handle, config_state, audit_logger_state, params), fields(pattern = %params.pattern, path = %params.path))]
pub async fn search_code_command(
    app_handle: AppHandle,
    config_state: State<'_, Arc<StdRwLock<Config>>>,
    audit_logger_state: State<'_, Arc<crate::utils::audit_logger::AuditLogger>>,
    params: SearchCodeParams,
) -> Result<SearchCodeResult, AppError> {
    audit_log(&audit_logger_state, "search_code", &serde_json::to_value(¶ms)?).await;

    let rg_exe_path = get_rg_path()?;
    let config_guard = config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock error: {}",e)))?;
    debug!("Ripgrep search_code with params: {:?}", params);

    let search_dir_str = if params.path.is_empty() || params.path == "." {
        config_guard.files_root.to_str().unwrap_or(".").to_string()
    } else {
        params.path.clone()
    };

    let search_path_validated = validate_and_normalize_path(&search_dir_str, &config_guard, true, false)?;
    let files_root_for_stripping = config_guard.files_root.clone();
    drop(config_guard);

    let mut rg_args = Vec::new();
    rg_args.push("--json".to_string());
    if params.line_numbers { rg_args.push("--line-number".to_string()); }
    if params.fixed_strings { rg_args.push("-F".to_string()); }
    if params.case_sensitive { rg_args.push("-s".to_string()); }
    else if params.ignore_case { rg_args.push("-i".to_string()); }
    if let Some(context) = params.context_lines { if context > 0 { rg_args.push("-C".to_string()); rg_args.push(context.to_string()); }}
    if let Some(glob) = ¶ms.file_pattern { if !glob.is_empty() { rg_args.push("-g".to_string()); rg_args.push(glob.clone()); }}
    if let Some(depth) = params.max_depth { rg_args.push("--max-depth".to_string()); rg_args.push(depth.to_string()); }
    rg_args.push("--max-count".to_string()); rg_args.push(params.max_results.to_string());
    if params.include_hidden { rg_args.push("--hidden".to_string()); }
    rg_args.push(params.pattern.clone());
    rg_args.push(search_path_validated.to_string_lossy().to_string());

    let shell_scope = app_handle.shell().scope();
    if !shell_scope.is_allowed(&rg_exe_path.to_string_lossy(), &rg_args) {
        warn!(command = %rg_exe_path.display(), args = ?rg_args, "Ripgrep command execution not allowed by shell scope.");
        return Err(AppError::CommandBlocked("Execution of ripgrep (rg) not permitted by shell scope.".to_string()));
    }

    let start_time = std::time::Instant::now();
    let command_future = app_handle.shell().command(rg_exe_path.to_string_lossy().to_string())
        .args(rg_args.clone()) // Clone args for logging/error reporting
        .current_dir(&search_path_validated)
        .output();

    let timeout_duration = Duration::from_millis(params.timeout_ms.unwrap_or(30000));

    match timeout(timeout_duration, command_future).await {
        Ok(Ok(output)) => {
            let elapsed_ms = start_time.elapsed().as_millis() as u64;
            let mut error_message_opt: Option<String> = None;

            if !output.status.success() && output.status.code() != Some(1) {
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                error!("Ripgrep command failed with status {:?}: {}", output.status, stderr);
                if output.stdout.is_empty() {
                    return Err(AppError::RipgrepError(format!("Ripgrep failed (status: {:?}): {}", output.status, stderr)));
                }
                error_message_opt = Some(format!("Ripgrep reported errors (status: {:?}): {}", output.status, stderr));
            }
            if !output.stderr.is_empty() && error_message_opt.is_none() {
                 let stderr_str = String::from_utf8_lossy(&output.stderr).to_string();
                 if !stderr_str.trim().is_empty() { error_message_opt = Some(format!("Ripgrep stderr: {}", stderr_str)); }
            }

            let stdout = String::from_utf8(output.stdout).map_err(|e| AppError::RipgrepError(format!("Ripgrep output not UTF-8: {}", e)))?;
            let mut matches = Vec::new();
            let mut matched_lines_count = 0;

            for line_str in stdout.lines() {
                if line_str.trim().is_empty() { continue; }
                match serde_json::from_str::<serde_json::Value>(line_str) {
                    Ok(json_val) => {
                        let entry_type = json_val.get("type").and_then(|t| t.as_str());
                        if let Some(data) = json_val.get("data") {
                            let path_text_abs = data.get("path").and_then(|p| p.get("text")).and_then(|t| t.as_str()).unwrap_or_default();
                            let line_num = data.get("line_number").and_then(|n| n.as_u64()).unwrap_or(0);
                            let mut match_line_content = String::new();

                            if entry_type == Some("match") {
                                if let Some(submatches_array) = data.get("submatches").and_then(|s| s.as_array()) {
                                    for submatch_obj in submatches_array {
                                        if let Some(match_text_val) = submatch_obj.get("match").and_then(|m| m.get("text")) {
                                            match_line_content.push_str(match_text_val.as_str().unwrap_or(""));
                                        }
                                    }
                                }
                                matched_lines_count += 1;
                            } else if entry_type == Some("context") {
                                 if let Some(lines_text_val) = data.get("lines").and_then(|l| l.get("text")) {
                                    match_line_content.push_str(lines_text_val.as_str().unwrap_or(""));
                                }
                            } else { continue; }

                            let absolute_match_path = PathBuf::from(path_text_abs);
                            let display_path = match absolute_match_path.strip_prefix(&files_root_for_stripping) {
                                Ok(p) => p.to_string_lossy().into_owned(),
                                Err(_) => path_text_abs.to_string(),
                            };
                            matches.push(RipgrepMatch { file: display_path, line: line_num, match_text: match_line_content.trim_end().to_string() });
                        }
                    }
                    Err(e) => { warn!(error = %e, line = %line_str, "Failed to parse ripgrep JSON line"); }
                }
            }
            Ok(SearchCodeResult { matches, stats: SearchStats { matched_lines: matched_lines_count, elapsed_ms }, timed_out: false, error_message: error_message_opt })
        },
        Ok(Err(e)) => {
            error!("Error executing ripgrep command via tauri-plugin-shell: {:?}", e);
            Err(AppError::RipgrepError(format!("Shell execution error for ripgrep: {:?}", e)))
        }
        Err(_) => {
            let elapsed_ms = start_time.elapsed().as_millis() as u64;
            warn!(pattern = %params.pattern, path = %params.path, timeout = timeout_duration.as_millis(), "Ripgrep search timed out");
            Ok(SearchCodeResult { matches: vec![], stats: SearchStats { matched_lines: 0, elapsed_ms }, timed_out: true, error_message: Some("Search operation timed out.".to_string()) })
        }
    }
}