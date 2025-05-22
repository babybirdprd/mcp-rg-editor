// FILE: src-tauri/src/mcp/tool_impl/ripgrep.rs
// IMPORTANT NOTE: Create this new file.
use crate::config::Config;
use crate::error::AppError;
use crate::mcp::handler::ToolDependencies;
use crate::utils::path_utils::validate_and_normalize_path;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::Manager;
use tauri_plugin_shell::ShellExt;
use tokio::time::{timeout, Duration};
use tracing::{debug, error, instrument, warn};

// --- MCP Specific Parameter Struct ---
#[derive(Debug, Clone, Deserialize)]
pub struct SearchCodeParamsMCP {
    pub pattern: String,
    #[serde(default)]
    pub path: String,
    #[serde(default, alias = "fixedStrings")]
    pub fixed_strings: bool,
    #[serde(default, alias = "ignoreCase")]
    pub ignore_case: bool,
    #[serde(default)]
    pub case_sensitive: bool,
    #[serde(default = "default_true_mcp")]
    pub line_numbers: bool,
    #[serde(alias = "contextLines")]
    pub context_lines: Option<usize>,
    #[serde(default, alias = "filePattern")]
    pub file_pattern: Option<String>,
    #[serde(alias = "maxDepth")]
    pub max_depth: Option<usize>,
    #[serde(default = "default_usize_1000_mcp")]
    pub max_results: usize,
    #[serde(default, alias = "includeHidden")]
    pub include_hidden: bool,
    #[serde(default, rename = "timeoutMs")]
    pub timeout_ms: Option<u64>,
}
fn default_true_mcp() -> bool { true }
fn default_usize_1000_mcp() -> usize { 1000 }

// --- MCP Specific Result Structs ---
#[derive(Debug, Clone, Serialize)]
pub struct RipgrepMatchMCP {
    pub file: String,
    pub line: u64,
    pub match_text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchStatsMCP {
    pub matched_lines: usize,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchCodeResultMCP {
    pub matches: Vec<RipgrepMatchMCP>,
    pub stats: SearchStatsMCP,
    pub timed_out: bool,
    pub error_message: Option<String>,
}

fn get_rg_path_mcp() -> Result<PathBuf, AppError> {
    which::which("rg").map_err(|e| AppError::RipgrepError(format!("rg not found: {}. Please install ripgrep.", e)))
}

#[instrument(skip(deps, params), fields(pattern = %params.pattern, path = %params.path))]
pub async fn mcp_search_code(
    deps: &ToolDependencies,
    params: SearchCodeParamsMCP,
) -> Result<SearchCodeResultMCP, AppError> {
    let rg_exe_path = get_rg_path_mcp()?;
    let config_guard = deps.config_state.read().map_err(|e| AppError::ConfigError(format!("Config lock: {}", e)))?;
    debug!("MCP Tool: search_code with params: {:?}", params);

    let search_dir_str = if params.path.is_empty() || params.path == "." {
        config_guard.files_root.to_str().unwrap_or(".").to_string()
    } else { params.path.clone() };

    let search_path_validated = validate_and_normalize_path(&search_dir_str, &config_guard, true, false)?;
    let files_root_for_stripping = config_guard.files_root.clone();
    drop(config_guard);

    let mut rg_args = Vec::new();
    rg_args.push("--json".to_string());
    if params.line_numbers { rg_args.push("--line-number".to_string()); }
    // ... (construct all other rg_args as in ripgrep_commands.rs) ...
    if params.fixed_strings { rg_args.push("-F".to_string()); }
    if params.case_sensitive { rg_args.push("-s".to_string()); }
    else if params.ignore_case { rg_args.push("-i".to_string()); }
    if let Some(context) = params.context_lines { if context > 0 { rg_args.push("-C".to_string()); rg_args.push(context.to_string()); }}
    if let Some(glob) = params.file_pattern { if !glob.is_empty() { rg_args.push("-g".to_string()); rg_args.push(glob.clone()); }}
    if let Some(depth) = params.max_depth { rg_args.push("--max-depth".to_string()); rg_args.push(depth.to_string()); }
    rg_args.push("--max-count".to_string()); rg_args.push(params.max_results.to_string());
    if params.include_hidden { rg_args.push("--hidden".to_string()); }
    rg_args.push(params.pattern.clone());
    rg_args.push(search_path_validated.to_string_lossy().to_string());


    if !deps.app_handle.shell().scope().is_allowed(&rg_exe_path.to_string_lossy(), &rg_args) {
        return Err(AppError::CommandBlocked("rg execution not allowed by shell scope".to_string()));
    }

    let start_time = std::time::Instant::now();
    let command_future = deps.app_handle.shell().command(rg_exe_path.to_string_lossy().to_string())
        .args(rg_args).current_dir(&search_path_validated).output();
    let timeout_duration = Duration::from_millis(params.timeout_ms.unwrap_or(30000));

    match timeout(timeout_duration, command_future).await {
        Ok(Ok(output)) => {
            let elapsed_ms = start_time.elapsed().as_millis() as u64;
            let mut error_message_opt: Option<String> = None;
            if !output.status.success() && output.status.code() != Some(1) {
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                if output.stdout.is_empty() { return Err(AppError::RipgrepError(format!("rg failed (status: {:?}): {}", output.status, stderr))); }
                error_message_opt = Some(format!("rg errors (status: {:?}): {}", output.status, stderr));
            }
            if !output.stderr.is_empty() && error_message_opt.is_none() && !String::from_utf8_lossy(&output.stderr).trim().is_empty() {
                 error_message_opt = Some(format!("rg stderr: {}", String::from_utf8_lossy(&output.stderr)));
            }

            let stdout = String::from_utf8(output.stdout).map_err(|e| AppError::RipgrepError(format!("rg output not UTF-8: {}", e)))?;
            let mut matches = Vec::new();
            let mut matched_lines_count = 0;

            for line_str in stdout.lines() {
                if line_str.trim().is_empty() { continue; }
                match serde_json::from_str::<serde_json::Value>(line_str) {
                    Ok(json_val) => {
                        let entry_type = json_val.get("type").and_then(|t| t.as_str());
                        if let Some(data) = json_val.get("data") {
                            let path_abs = data.get("path").and_then(|p|p.get("text")).and_then(|t|t.as_str()).unwrap_or_default();
                            let line_num = data.get("line_number").and_then(|n|n.as_u64()).unwrap_or(0);
                            let mut match_text = String::new();
                            if entry_type == Some("match") {
                                if let Some(subs) = data.get("submatches").and_then(|s|s.as_array()) {
                                    for sub in subs { if let Some(txt) = sub.get("match").and_then(|m|m.get("text")) { match_text.push_str(txt.as_str().unwrap_or(""));}}
                                }
                                matched_lines_count +=1;
                            } else if entry_type == Some("context") {
                                if let Some(txt) = data.get("lines").and_then(|l|l.get("text")) { match_text.push_str(txt.as_str().unwrap_or(""));}
                            } else { continue; }
                            let display_path = PathBuf::from(path_abs).strip_prefix(&files_root_for_stripping).map(|p|p.to_string_lossy().into_owned()).unwrap_or_else(|_|path_abs.to_string());
                            matches.push(RipgrepMatchMCP { file: display_path, line: line_num, match_text: match_text.trim_end().to_string() });
                        }
                    }
                    Err(e) => { warn!(error = %e, line = %line_str, "Failed to parse rg JSON line"); }
                }
            }
            Ok(SearchCodeResultMCP { matches, stats: SearchStatsMCP { matched_lines: matched_lines_count, elapsed_ms }, timed_out: false, error_message: error_message_opt })
        },
        Ok(Err(e)) => Err(AppError::RipgrepError(format!("Shell exec error for rg: {:?}", e))),
        Err(_) => {
            let elapsed_ms = start_time.elapsed().as_millis() as u64;
            Ok(SearchCodeResultMCP { matches: vec![], stats: SearchStatsMCP { matched_lines: 0, elapsed_ms }, timed_out: true, error_message: Some("Search timed out.".to_string()) })
        }
    }
}