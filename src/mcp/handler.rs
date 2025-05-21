use crate::config::Config;
use crate::mcp::schemas::*;
use crate::tools::{
    edit_tool::{EditManager, EditBlockParams, EditBlockResult},
    filesystem_tool::{
        FilesystemManager, ReadFileParams, ReadMultipleFilesParams, WriteFileParams, CreateDirectoryParams,
        ListDirectoryParams, MoveFileParams, SearchFilesParams, GetFileInfoParams, FileContent,
        FileOperationResult, ListDirectoryResult, SearchFilesResult, FileInfoResult
    },
    process_tool::{ProcessManager, KillProcessParams, ProcessInfo, KillProcessResult},
    ripgrep_tool::{RipgrepSearcher, SearchCodeParams, SearchCodeResult},
    terminal_tool::{
        TerminalManager, ExecuteCommandParams, ReadOutputParams, ForceTerminateParams,
        ExecuteCommandResult, ReadOutputResult, ForceTerminateResult, SessionInfo
    },
};
use crate::utils::audit_logger::AuditLogger;

use async_trait::async_trait;
use rust_mcp_sdk::mcp_server::server_runtime::ServerRuntimeContext; // Corrected import path
use rust_mcp_schema::{
    CallToolRequest, CallToolResult, ListToolsRequest, ListToolsResult, Tool, Content,
    schema_utils::CallToolError, RpcError, RpcErrorCode,
};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::{Arc, RwLock as StdRwLock};
use regex::Regex;
use tracing::{debug, error, info, instrument, warn};


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
        let audit_logger_arc = Arc::new(AuditLogger::new(config_arc.clone()));
        let filesystem_manager_arc = Arc::new(FilesystemManager::new(config_arc.clone()));
        
        Self {
            config: config_arc.clone(),
            filesystem_manager: filesystem_manager_arc.clone(),
            terminal_manager: Arc::new(TerminalManager::new(config_arc.clone())),
            process_manager: Arc::new(ProcessManager::new(config_arc.clone())),
            ripgrep_searcher: Arc::new(RipgrepSearcher::new(config_arc.clone())),
            edit_manager: Arc::new(EditManager::new(config_arc.clone(), filesystem_manager_arc, audit_logger_arc.clone())), // Pass audit_logger to EditManager
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

fn create_json_call_tool_result(value: Value) -> Result<CallToolResult, CallToolError> {
    let content_item = Content::Other {
        type_: "json".to_string(),
        data: Some(value),
        text: None,
        mime_type: None,
        resource_id: None,
        name: None,
        size: None,
        created_at: None,
        updated_at: None,
        meta: None,
    };
    Ok(CallToolResult::new(vec![content_item], None))
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
            Tool::new("get_config".to_string(), Some("Get the complete server configuration as JSON. Includes blockedCommands, defaultShell, allowedDirectories, fileReadLineLimit, fileWriteLineLimit, etc.".to_string()), Some(get_config_schema()), None),
            Tool::new("set_config_value".to_string(), Some("Set a specific configuration value by key. WARNING: Use in a separate chat. Changes are in-memory unless persistence is configured. Keys: blockedCommands, defaultShell, allowedDirectories, fileReadLineLimit, fileWriteLineLimit.".to_string()), Some(set_config_value_schema()), None),
            Tool::new("read_file".to_string(), Some("Read content of a local file or URL. Supports line offset/length for local text files. Images returned as base64.".to_string()), Some(read_file_schema()), None),
            Tool::new("read_multiple_files".to_string(), Some("Read multiple local files. Images returned as base64.".to_string()), Some(read_multiple_files_schema()), None),
            Tool::new("write_file".to_string(), Some("Write/append content to a file. Enforces line limits; chunk large writes.".to_string()), Some(write_file_schema()), None),
            Tool::new("create_directory".to_string(), Some("Create directories, including nested ones.".to_string()), Some(create_directory_schema()), None),
            Tool::new("list_directory".to_string(), Some("List directory contents with [FILE]/[DIR] prefixes.".to_string()), Some(list_directory_schema()), None),
            Tool::new("move_file".to_string(), Some("Move or rename files or directories.".to_string()), Some(move_file_schema()), None),
            Tool::new("search_files".to_string(), Some("Find files by name (substring match) with timeout.".to_string()), Some(search_files_schema()), None),
            Tool::new("get_file_info".to_string(), Some("Get metadata for a file or directory (size, timestamps, permissions).".to_string()), Some(get_file_info_schema()), None),
            Tool::new("search_code".to_string(), Some("Search code with Ripgrep (regex, file types, context, timeout).".to_string()), Some(search_code_schema()), None),
            Tool::new("edit_block".to_string(), Some("Apply targeted text replacements. Supports expected_replacements, fuzzy matching feedback.".to_string()), Some(edit_block_schema()), None),
            Tool::new("execute_command".to_string(), Some("Run terminal commands with timeout, background execution, and shell selection.".to_string()), Some(execute_command_schema()), None),
            Tool::new("read_output".to_string(), Some("Get new output from a running command session.".to_string()), Some(read_output_schema()), None),
            Tool::new("force_terminate".to_string(), Some("Stop a running command session by its session_id.".to_string()), Some(force_terminate_schema()), None),
            Tool::new("list_sessions".to_string(), Some("List active command sessions with PIDs and runtime.".to_string()), Some(list_sessions_schema()), None),
            Tool::new("list_processes".to_string(), Some("List system processes (PID, name, CPU/memory).".to_string()), Some(list_processes_schema()), None),
            Tool::new("kill_process".to_string(), Some("Terminate a system process by PID.".to_string()), Some(kill_process_schema()), None),
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
                            CallToolError::new(RpcError::new(RpcErrorCode::InvalidParams, format!("Invalid arguments for {}: {}", tool_name, e), None))
                        })?;
                    let result: $result_type = self.managers.$manager_field.$method(&params).await?;
                    let value_result = serde_json::to_value(&result)
                        .map_err(|e| CallToolError::new(RpcError::new(RpcErrorCode::InternalError, format!("Failed to serialize result: {}", e), None)))?;
                    create_json_call_tool_result(value_result)
                }
            };
        }
        macro_rules! handle_tool_no_params {
             ($manager_field:ident . $method:ident returning $result_type:ty) => {
                {
                    let result: $result_type = self.managers.$manager_field.$method().await?;
                    let value_result = serde_json::to_value(&result)
                        .map_err(|e| CallToolError::new(RpcError::new(RpcErrorCode::InternalError, format!("Failed to serialize result: {}", e), None)))?;
                    create_json_call_tool_result(value_result)
                }
            };
        }
        
        match tool_name {
            "get_config" => {
                let config_guard = self.managers.config.read().map_err(|e| CallToolError::new(RpcError::new(RpcErrorCode::InternalError, format!("Failed to read config lock: {}", e), None)))?;
                let current_config_data = config_guard.clone(); // Clone the data, not the guard
                drop(config_guard); // Explicitly drop guard

                let response_config = json!({
                    "files_root": current_config_data.files_root.display().to_string(),
                    "allowed_directories": current_config_data.allowed_directories.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
                    "blocked_commands": current_config_data.blocked_commands.iter().map(|r| r.as_str().to_string()).collect::<Vec<_>>(),
                    "default_shell": current_config_data.default_shell,
                    "file_read_line_limit": current_config_data.file_read_line_limit,
                    "file_write_line_limit": current_config_data.file_write_line_limit,
                    "log_level": current_config_data.log_level,
                    "transport_mode": format!("{:?}", current_config_data.transport_mode),
                    "sse_host": current_config_data.sse_host,
                    "sse_port": current_config_data.sse_port,
                });
                create_json_call_tool_result(response_config)
            },
            "set_config_value" => {
                let params: SetConfigValueParams = serde_json::from_value(args_value)
                    .map_err(|e| CallToolError::new(RpcError::new(RpcErrorCode::InvalidParams, e.to_string(), None)))?;
                
                let mut config_guard = self.managers.config.write().map_err(|e| CallToolError::new(RpcError::new(RpcErrorCode::InternalError, format!("Failed to write config lock: {}", e), None)))?;
                let key = params.key.as_str();
                let value_to_set = params.value;

                let mut update_applied = true;
                match key {
                    "allowedDirectories" => {
                        if let Some(arr_val) = value_to_set.as_array() {
                            config_guard.allowed_directories = arr_val.iter()
                                .filter_map(|v| v.as_str().map(PathBuf::from))
                                .collect();
                        } else if let Some(str_val) = value_to_set.as_str() {
                            config_guard.allowed_directories = str_val.split(',').map(|s| PathBuf::from(s.trim())).collect();
                        } else { update_applied = false; }
                    },
                    "blockedCommands" => {
                         if let Some(arr_val) = value_to_set.as_array() {
                            config_guard.blocked_commands = arr_val.iter()
                                .filter_map(|v| v.as_str())
                                .filter_map(|s| Regex::new(&format!(r"^(?:[a-zA-Z_][a-zA-Z0-9_]*=[^ ]* )*{}(?:\s.*|$)", regex::escape(s))).ok())
                                .collect();
                        } else if let Some(str_val) = value_to_set.as_str() {
                             config_guard.blocked_commands = str_val.split(',').map(|s| s.trim()).filter(|s| !s.is_empty())
                                .filter_map(|s| Regex::new(&format!(r"^(?:[a-zA-Z_][a-zA-Z0-9_]*=[^ ]* )*{}(?:\s.*|$)", regex::escape(s))).ok())
                                .collect();
                        } else { update_applied = false; }
                    },
                    "defaultShell" => {
                        if let Some(str_val) = value_to_set.as_str() { config_guard.default_shell = Some(str_val.to_string()); } 
                        else { update_applied = false; warn!(key=key, "set_config_value: value for string key was not a string"); }
                    },
                    "fileReadLineLimit" => {
                        if let Some(num_val) = value_to_set.as_u64() { config_guard.file_read_line_limit = num_val as usize; }
                        else { update_applied = false; warn!(key=key, "set_config_value: value for u64 key was not u64");}
                    },
                     "fileWriteLineLimit" => {
                        if let Some(num_val) = value_to_set.as_u64() { config_guard.file_write_line_limit = num_val as usize; }
                        else { update_applied = false; warn!(key=key, "set_config_value: value for u64 key was not u64");}
                    },
                    _ => {
                        update_applied = false;
                        warn!(key=key, "set_config_value: Unknown or unhandled config key");
                        return Err(CallToolError::new(RpcError::new(RpcErrorCode::InvalidParams, format!("Unknown or read-only config key: {}", key), None)));
                    }
                }
                drop(config_guard);

                if update_applied {
                     Ok(CallToolResult::text_content(format!("Successfully set config key '{}'. Change is in-memory for current session.", key), None)?)
                } else {
                    Err(CallToolError::new(RpcError::new(RpcErrorCode::InvalidParams, format!("Invalid value type for config key '{}'", key), None)))
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
            "edit_block" => handle_tool_with_params!(edit_manager.edit_block::<EditBlockParams> returning EditBlockResult),
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