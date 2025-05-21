use crate::config::Config;
// Removed AppError import as it's converted via From trait for CallToolError
use crate::mcp::schemas::*;
use crate::tools::{
    edit_tool::{EditManager, EditBlockParams},
    filesystem_tool::{
        FilesystemManager, ReadFileParams, ReadMultipleFilesParams, WriteFileParams, CreateDirectoryParams,
        ListDirectoryParams, MoveFileParams, SearchFilesParams, GetFileInfoParams, FileContent, // Added FileContent for result type
        FileOperationResult, ListDirectoryResult, SearchFilesResult, FileInfoResult // Added other result types
    },
    process_tool::{ProcessManager, KillProcessParams, ProcessInfo, KillProcessResult}, // Added result types
    ripgrep_tool::{RipgrepSearcher, SearchCodeParams, SearchCodeResult}, // Added result type
    terminal_tool::{
        TerminalManager, ExecuteCommandParams, ReadOutputParams, ForceTerminateParams,
        ExecuteCommandResult, ReadOutputResult, ForceTerminateResult, SessionInfo // Added result types
    },
};
use crate::utils::audit_logger::AuditLogger;

use async_trait::async_trait;
use rust_mcp_sdk::mcp_server::{ServerHandler, ServerRuntimeContext}; // Corrected ServerRuntimeContext import
use rust_mcp_schema::{
    CallToolRequest, CallToolResult, ListToolsRequest, ListToolsResult, Tool,
    schema_utils::CallToolError,
};
use serde_json::{Value, json};
use std::path::PathBuf; // Added for set_config_value
use std::sync::{Arc, RwLock as StdRwLock};
use regex::Regex; // Added for set_config_value
use tracing::{debug, error, info, instrument}; // Added warn

// AppManagers now derives Debug
#[derive(Debug)]
pub struct AppManagers {
    config: Arc<StdRwLock<Config>>,
    filesystem_manager: Arc<FilesystemManager>,
    terminal_manager: Arc<TerminalManager>,
    process_manager: Arc<ProcessManager>,
    ripgrep_searcher: Arc<RipgrepSearcher>,
    edit_manager: Arc<EditManager>,
    audit_logger: Arc<AuditLogger>,
}

impl AppManagers {
    pub fn new(initial_config: Config) -> Self {
        let config_arc = Arc::new(StdRwLock::new(initial_config));
        
        // Pass Arc<StdRwLock<Config>> to AuditLogger
        let audit_logger_arc = Arc::new(AuditLogger::new(config_arc.clone()));
        let filesystem_manager_arc = Arc::new(FilesystemManager::new(config_arc.clone()));
        
        Self {
            config: config_arc.clone(),
            filesystem_manager: filesystem_manager_arc.clone(),
            terminal_manager: Arc::new(TerminalManager::new(config_arc.clone())),
            process_manager: Arc::new(ProcessManager::new(config_arc.clone())),
            ripgrep_searcher: Arc::new(RipgrepSearcher::new(config_arc.clone())),
            edit_manager: Arc::new(EditManager::new(config_arc.clone(), filesystem_manager_arc)),
            audit_logger: audit_logger_arc,
        }
    }
}

#[derive(Debug)]
pub struct EnhancedServerHandler {
   managers: Arc<AppManagers>,
}

impl EnhancedServerHandler {
    pub fn new(initial_config: Config) -> Self {
        Self {
            managers: Arc::new(AppManagers::new(initial_config)),
        }
    }
}

#[async_trait]
impl ServerHandler for EnhancedServerHandler {
    #[instrument(skip(self, _request, _runtime))]
    async fn handle_list_tools_request(
        &self,
        _request: ListToolsRequest,
        _runtime: &ServerRuntimeContext,
    ) -> Result<ListToolsResult, rust_mcp_schema::RpcError> {
        info!("Handling list_tools request");
        let tools = vec![
            Tool::new("get_config", "Get the complete server configuration as JSON. Includes blockedCommands, defaultShell, allowedDirectories, fileReadLineLimit, fileWriteLineLimit, etc.", Some(get_config_schema())),
            Tool::new("set_config_value", "Set a specific configuration value by key. WARNING: Use in a separate chat. Changes are in-memory unless persistence is configured. Keys: blockedCommands, defaultShell, allowedDirectories, fileReadLineLimit, fileWriteLineLimit.", Some(set_config_value_schema())),
            Tool::new("read_file", "Read content of a local file or URL. Supports line offset/length for local text files. Images returned as base64.", Some(read_file_schema())),
            Tool::new("read_multiple_files", "Read multiple local files. Images returned as base64.", Some(read_multiple_files_schema())),
            Tool::new("write_file", "Write/append content to a file. Enforces line limits; chunk large writes.", Some(write_file_schema())),
            Tool::new("create_directory", "Create directories, including nested ones.", Some(create_directory_schema())),
            Tool::new("list_directory", "List directory contents with [FILE]/[DIR] prefixes.", Some(list_directory_schema())),
            Tool::new("move_file", "Move or rename files or directories.", Some(move_file_schema())),
            Tool::new("search_files", "Find files by name (substring match) with timeout.", Some(search_files_schema())),
            Tool::new("get_file_info", "Get metadata for a file or directory (size, timestamps, permissions).", Some(get_file_info_schema())),
            Tool::new("search_code", "Search code with Ripgrep (regex, file types, context, timeout).", Some(search_code_schema())),
            Tool::new("edit_block", "Apply targeted text replacements. Supports expected_replacements, fuzzy matching feedback.", Some(edit_block_schema())),
            Tool::new("execute_command", "Run terminal commands with timeout, background execution, and shell selection.", Some(execute_command_schema())),
            Tool::new("read_output", "Get new output from a running command session.", Some(read_output_schema())),
            Tool::new("force_terminate", "Stop a running command session by its session_id.", Some(force_terminate_schema())),
            Tool::new("list_sessions", "List active command sessions with PIDs and runtime.", Some(list_sessions_schema())),
            Tool::new("list_processes", "List system processes (PID, name, CPU/memory).", Some(list_processes_schema())),
            Tool::new("kill_process", "Terminate a system process by PID.", Some(kill_process_schema())),
        ];
        Ok(ListToolsResult { tools, meta: None, next_cursor: None })
    }

    #[instrument(skip(self, request, _runtime), fields(tool_name = %request.params.name))]
    async fn handle_call_tool_request(
        &self,
        request: CallToolRequest,
        _runtime: &ServerRuntimeContext,
    ) -> Result<CallToolResult, CallToolError> {
        let tool_name = request.params.name.as_str();
        info!("Handling call_tool request for: {}", tool_name);
        
        let args_value = request.params.arguments.clone().unwrap_or_else(|| json!({}));
        self.managers.audit_logger.log_tool_call(tool_name, &args_value).await;

        macro_rules! handle_tool_with_params {
            ($manager_field:ident . $method:ident :: <$param_type:ty> returning $result_type:ty) => {
                {
                    let params: $param_type = serde_json::from_value(args_value.clone())
                        .map_err(|e| {
                            error!(error = %e, tool = tool_name, "Invalid params for tool");
                            CallToolError::invalid_params(format!("Invalid arguments for {}: {}", tool_name, e))
                        })?;
                    let result: $result_type = self.managers.$manager_field.$method(&params).await?;
                    let value_result = serde_json::to_value(&result).map_err(|e| CallToolError::internal_error(format!("Failed to serialize result: {}", e)))?;
                    Ok(CallToolResult::success(value_result)?)
                }
            };
        }
        macro_rules! handle_tool_no_params {
             ($manager_field:ident . $method:ident returning $result_type:ty) => {
                {
                    let result: $result_type = self.managers.$manager_field.$method().await?;
                    let value_result = serde_json::to_value(&result).map_err(|e| CallToolError::internal_error(format!("Failed to serialize result: {}", e)))?;
                    Ok(CallToolResult::success(value_result)?)
                }
            };
        }
        
        match tool_name {
            "get_config" => {
                let config_guard = self.managers.config.read().map_err(|e| CallToolError::internal_error(format!("Failed to read config lock: {}", e)))?;
                let current_config_data = config_guard.clone();
                drop(config_guard);
                let response_config = json!({
                    "files_root": current_config_data.files_root,
                    "allowed_directories": current_config_data.allowed_directories,
                    "blocked_commands": current_config_data.blocked_commands.iter().map(|r| r.as_str().to_string()).collect::<Vec<_>>(),
                    "default_shell": current_config_data.default_shell,
                    "file_read_line_limit": current_config_data.file_read_line_limit,
                    "file_write_line_limit": current_config_data.file_write_line_limit,
                });
                Ok(CallToolResult::success(response_config)?)
            },
            "set_config_value" => {
                let params: SetConfigValueParams = serde_json::from_value(args_value)
                    .map_err(|e| CallToolError::invalid_params(e.to_string()))?;
                
                let mut config_guard = self.managers.config.write().map_err(|e| CallToolError::internal_error(format!("Failed to write config lock: {}", e)))?;
                let key = params.key.as_str();
                let value_to_set = params.value;

                let mut update_applied = true;
                match key {
                    "allowedDirectories" => {
                        if let Some(arr_val) = value_to_set.as_array() {
                            config_guard.allowed_directories = arr_val.iter()
                                .filter_map(|v| v.as_str().map(PathBuf::from))
                                .collect();
                        } else if let Some(str_val) = value_to_set.as_str() { // Handle single string input for arrays
                            config_guard.allowed_directories = vec![PathBuf::from(str_val)];
                        } else { update_applied = false; }
                    },
                    "blockedCommands" => {
                         if let Some(arr_val) = value_to_set.as_array() {
                            config_guard.blocked_commands = arr_val.iter()
                                .filter_map(|v| v.as_str())
                                .filter_map(|s| Regex::new(&format!(r"^(?:[a-zA-Z_][a-zA-Z0-9_]*=[^ ]* )*{}(?:\s.*|$)", regex::escape(s))).ok())
                                .collect();
                        } else if let Some(str_val) = value_to_set.as_str() { // Handle single string input for arrays
                             config_guard.blocked_commands = vec![str_val].iter()
                                .filter_map(|s| Regex::new(&format!(r"^(?:[a-zA-Z_][a-zA-Z0-9_]*=[^ ]* )*{}(?:\s.*|$)", regex::escape(s))).ok())
                                .collect();
                        } else { update_applied = false; }
                    },
                    "defaultShell" => {
                        if let Some(str_val) = value_to_set.as_str() { config_guard.default_shell = Some(str_val.to_string()); } 
                        else { update_applied = false; tracing::warn!(key=key, "set_config_value: value for string key was not a string"); }
                    },
                    "fileReadLineLimit" => {
                        if let Some(num_val) = value_to_set.as_u64() { config_guard.file_read_line_limit = num_val as usize; }
                        else { update_applied = false; tracing::warn!(key=key, "set_config_value: value for u64 key was not u64");}
                    },
                     "fileWriteLineLimit" => {
                        if let Some(num_val) = value_to_set.as_u64() { config_guard.file_write_line_limit = num_val as usize; }
                        else { update_applied = false; tracing::warn!(key=key, "set_config_value: value for u64 key was not u64");}
                    },
                    _ => {
                        update_applied = false;
                        tracing::warn!(key=key, "set_config_value: Unknown or unhandled config key");
                        return Err(CallToolError::invalid_params(format!("Unknown or read-only config key: {}", key)));
                    }
                }
                drop(config_guard);

                if update_applied {
                     Ok(CallToolResult::text_content(format!("Successfully set config key '{}'. Change is in-memory.", key), None)?)
                } else {
                    Err(CallToolError::invalid_params(format!("Invalid value type for config key '{}'", key)))
                }
            },
            "read_file" => handle_tool_with_params!(filesystem_manager.read_file::<ReadFileParams> returning FileContent),
            "read_multiple_files" => handle_tool_with_params!(filesystem_manager.read_multiple_files::<ReadMultipleFilesParams> returning Vec<FileContent>),
            "write_file" => handle_tool_with_params!(filesystem_manager.write_file::<WriteFileParams> returning FileOperationResult),
            "create_directory" => handle_tool_with_params!(filesystem_manager.create_directory::<CreateDirectoryParams> returning FileOperationResult),
            "list_directory" => handle_tool_with_params!(filesystem_manager.list_directory::<ListDirectoryParams> returning ListDirectoryResult),
            "move_file" => handle_tool_with_params!(filesystem_manager.move_file::<MoveFileParams> returning FileOperationResult),
            "search_files" => handle_tool_with_params!(filesystem_manager.search_files::<SearchFilesParams> returning SearchFilesResult),
            "get_file_info" => handle_tool_with_params!(filesystem_manager.get_file_info::<GetFileInfoParams> returning FileInfoResult),
            "search_code" => handle_tool_with_params!(ripgrep_searcher.search_code::<SearchCodeParams> returning SearchCodeResult),
            "edit_block" => handle_tool_with_params!(edit_manager.edit_block::<EditBlockParams> returning crate::tools::edit_tool::EditBlockResult),
            "execute_command" => handle_tool_with_params!(terminal_manager.execute_command::<ExecuteCommandParams> returning ExecuteCommandResult),
            "read_output" => handle_tool_with_params!(terminal_manager.read_output::<ReadOutputParams> returning ReadOutputResult),
            "force_terminate" => handle_tool_with_params!(terminal_manager.force_terminate::<ForceTerminateParams> returning ForceTerminateResult),
            "list_sessions" => handle_tool_no_params!(terminal_manager.list_sessions returning Vec<SessionInfo>),
            "list_processes" => handle_tool_no_params!(process_manager.list_processes returning Vec<ProcessInfo>),
            "kill_process" => handle_tool_with_params!(process_manager.kill_process::<KillProcessParams> returning KillProcessResult),
            _ => {
                error!("Unknown tool called: {}", tool_name);
                Err(CallToolError::unknown_tool(tool_name.to_string()))
            }
        }
    }
}