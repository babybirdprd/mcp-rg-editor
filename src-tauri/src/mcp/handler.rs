use crate::config::Config;
use crate::error::AppError;
use crate::mcp::schemas::*;
use crate::mcp::tool_impl;
use crate::utils::audit_logger::AuditLogger as AppAuditLogger;
use crate::utils::fuzzy_search_logger::FuzzySearchLogger as AppFuzzySearchLogger;
use crate::commands::terminal_commands::ActiveSessionsMap;
use sysinfo::System as SysinfoSystem;

use async_trait::async_trait;
use rust_mcp_sdk::McpServer;
use rust_mcp_sdk::mcp_server::ServerHandler;
use rust_mcp_schema::{
    CallToolRequest, CallToolResult, ListToolsRequest, ListToolsResult, Tool,
    CallToolResultContentItem, 
    TextContent, // MODIFIED: Removed JsonContent, TextContent is used.              
    schema_utils::CallToolError, RpcError, schema_utils::RpcErrorCodes, 
};
use serde_json::Value;
use std::sync::{Arc, RwLock as StdRwLock};
use tauri::{AppHandle, Manager};
use tokio::sync::Mutex as TokioMutex;
use tracing::{error, info, instrument};

#[derive(Clone)]
pub struct ToolDependencies {
    pub app_handle: AppHandle,
    pub config_state: Arc<StdRwLock<Config>>,
    pub audit_logger: Arc<AppAuditLogger>,
    pub fuzzy_search_logger: Arc<AppFuzzySearchLogger>,
    pub active_sessions_map: ActiveSessionsMap,
    pub sysinfo_state: Arc<TokioMutex<SysinfoSystem>>,
}

#[derive(Clone)]
pub struct EnhancedServerHandler {
   deps: ToolDependencies,
}

impl EnhancedServerHandler {
    pub fn new(app_handle: AppHandle, config_state: Arc<StdRwLock<Config>>) -> Self {
        let audit_logger = app_handle.state::<Arc<AppAuditLogger>>().inner().clone();
        let fuzzy_search_logger = app_handle.state::<Arc<AppFuzzySearchLogger>>().inner().clone();
        let active_sessions_map = app_handle.state::<ActiveSessionsMap>().inner().clone();
        let sysinfo_state = app_handle.state::<Arc<TokioMutex<SysinfoSystem>>>().inner().clone();

        Self {
            deps: ToolDependencies {
                app_handle,
                config_state,
                audit_logger,
                fuzzy_search_logger,
                active_sessions_map,
                sysinfo_state,
            },
        }
    }
}

fn mcp_call_tool_error_from_app_error(app_err: AppError, tool_name: &str) -> CallToolError {
    error!(error = %app_err, tool = tool_name, "Error during MCP tool execution");
    
    let (rpc_error_code_enum, message) = match app_err {
        AppError::InvalidInputArgument(ref msg) | 
        AppError::PathNotAllowed(ref msg) | 
        AppError::PathTraversal(ref msg) | 
        AppError::InvalidPath(ref msg) => (RpcErrorCodes::INVALID_PARAMS, msg.clone()),
        AppError::CommandBlocked(ref cmd_name) => {
            (RpcErrorCodes::INTERNAL_ERROR, format!("Command blocked (Server Code -32001): {}", cmd_name))
        },
        _ => (RpcErrorCodes::INTERNAL_ERROR, app_err.to_string()),
    };
    
    CallToolError::new(RpcError::new(rpc_error_code_enum, message, None))
}

fn create_mcp_json_call_tool_result(value: Value) -> Result<CallToolResult, CallToolError> {
    // MODIFIED: Reverted to serializing to string and using TextContent
    // as JsonContent variant is not available in the used schema version.
    let json_string = serde_json::to_string(&value)
        .map_err(|e| CallToolError::new(RpcError::new(RpcErrorCodes::INTERNAL_ERROR, format!("Failed to serialize result to JSON string: {}", e), None)))?;
    
    let content_item = CallToolResultContentItem::TextContent(TextContent::new(
        json_string,
        None, 
    ));
    Ok(CallToolResult { content: vec![content_item], meta: None, is_error: Some(false) })
}


#[async_trait]
impl ServerHandler for EnhancedServerHandler {
    #[instrument(skip(self, _request, _runtime))]
    async fn handle_list_tools_request(
        &self,
        _request: ListToolsRequest,
        _runtime: &dyn McpServer,
    ) -> Result<ListToolsResult, RpcError> {
        info!("MCP: Handling list_tools request");
        let tools = vec![
            Tool { name: "mcp_get_config".to_string(), description: Some("Get the MCP server's current runtime configuration.".to_string()), input_schema: get_mcp_config_schema()},
            Tool { name: "read_file".to_string(), description: Some("Read content of a local file or URL.".to_string()), input_schema: read_file_mcp_schema()},
            Tool { name: "write_file".to_string(), description: Some("Write/append content to a file.".to_string()), input_schema: write_file_mcp_schema()},
            Tool { name: "create_directory".to_string(), description: Some("Create directories, including nested ones.".to_string()), input_schema: create_directory_mcp_schema()},
            Tool { name: "list_directory".to_string(), description: Some("List directory contents.".to_string()), input_schema: list_directory_mcp_schema()},
            Tool { name: "move_file".to_string(), description: Some("Move or rename files or directories.".to_string()), input_schema: move_file_mcp_schema()},
            Tool { name: "get_file_info".to_string(), description: Some("Get metadata for a file or directory.".to_string()), input_schema: get_file_info_mcp_schema()},
            Tool { name: "read_multiple_files".to_string(), description: Some("Read multiple local files.".to_string()), input_schema: read_multiple_files_mcp_schema()},
            Tool { name: "search_files".to_string(), description: Some("Find files/dirs by name.".to_string()), input_schema: search_files_mcp_schema()},
            Tool { name: "search_code".to_string(), description: Some("Search code with Ripgrep.".to_string()), input_schema: search_code_mcp_schema()},
            Tool { name: "execute_command".to_string(), description: Some("Run terminal commands. Output is streamed via events if using Tauri UI; for MCP, initial output/status returned.".to_string()), input_schema: execute_command_mcp_schema()},
            Tool { name: "force_terminate_session".to_string(), description: Some("Stop a running command session by its ID.".to_string()), input_schema: force_terminate_mcp_schema()},
            Tool { name: "list_sessions".to_string(), description: Some("List active command sessions.".to_string()), input_schema: list_sessions_mcp_schema()},
            Tool { name: "read_session_output_status".to_string(), description: Some("Get status of a command session. For MCP, this might include buffered output if designed so.".to_string()), input_schema: read_session_output_status_mcp_schema()},
            Tool { name: "list_processes".to_string(), description: Some("List system processes.".to_string()), input_schema: list_processes_mcp_schema()},
            Tool { name: "kill_process".to_string(), description: Some("Terminate a system process by PID.".to_string()), input_schema: kill_process_mcp_schema()},
            Tool { name: "edit_block".to_string(), description: Some("Apply targeted text replacements in a file.".to_string()), input_schema: edit_block_mcp_schema()},
        ];
        Ok(ListToolsResult { tools, meta: None, next_cursor: None })
    }

    #[instrument(skip(self, request, _runtime), fields(tool_name = %request.params.name))]
    async fn handle_call_tool_request(
        &self,
        request: CallToolRequest,
        _runtime: &dyn McpServer,
    ) -> Result<CallToolResult, CallToolError> {
        let tool_name = request.params.name.as_str();
        let args_value = Value::Object(request.params.arguments.clone().unwrap_or_default());
        info!(tool_name = %tool_name, "MCP: Handling call_tool request");
        
        self.deps.audit_logger.log_command_call(&format!("mcp_{}", tool_name), &args_value).await;

        match tool_name {
            "mcp_get_config" => {
                let current_config_data = { 
                    let config_guard = self.deps.config_state.read()
                        .map_err(|e| CallToolError::new(RpcError::new(RpcErrorCodes::INTERNAL_ERROR, format!("Config lock error: {}", e), None)))?;
                    config_guard.clone()
                };
                let value_result = serde_json::to_value(current_config_data)
                    .map_err(|e| CallToolError::new(RpcError::new(RpcErrorCodes::INTERNAL_ERROR, format!("Failed to serialize config: {}", e), None)))?;
                create_mcp_json_call_tool_result(value_result)
            }
            "read_file" => {
                let params: tool_impl::filesystem::ReadFileParamsMCP = serde_json::from_value(args_value.clone())
                    .map_err(|e| CallToolError::new(RpcError::new(RpcErrorCodes::INVALID_PARAMS, e.to_string(), None)))?;
                let result = tool_impl::filesystem::mcp_read_file(&self.deps, params).await.map_err(|e| mcp_call_tool_error_from_app_error(e, tool_name))?;
                create_mcp_json_call_tool_result(serde_json::to_value(result).unwrap())
            }
            "write_file" => {
                let params: tool_impl::filesystem::WriteFileParamsMCP = serde_json::from_value(args_value.clone())
                    .map_err(|e| CallToolError::new(RpcError::new(RpcErrorCodes::INVALID_PARAMS, e.to_string(), None)))?;
                let result = tool_impl::filesystem::mcp_write_file(&self.deps, params).await.map_err(|e| mcp_call_tool_error_from_app_error(e, tool_name))?;
                create_mcp_json_call_tool_result(serde_json::to_value(result).unwrap())
            }
             "create_directory" => {
                let params: tool_impl::filesystem::CreateDirectoryParamsMCP = serde_json::from_value(args_value.clone())
                    .map_err(|e| CallToolError::new(RpcError::new(RpcErrorCodes::INVALID_PARAMS, e.to_string(), None)))?;
                let result = tool_impl::filesystem::mcp_create_directory(&self.deps, params).await.map_err(|e| mcp_call_tool_error_from_app_error(e, tool_name))?;
                create_mcp_json_call_tool_result(serde_json::to_value(result).unwrap())
            }
            "list_directory" => {
                let params: tool_impl::filesystem::ListDirectoryParamsMCP = serde_json::from_value(args_value.clone())
                    .map_err(|e| CallToolError::new(RpcError::new(RpcErrorCodes::INVALID_PARAMS, e.to_string(), None)))?;
                let result = tool_impl::filesystem::mcp_list_directory(&self.deps, params).await.map_err(|e| mcp_call_tool_error_from_app_error(e, tool_name))?;
                create_mcp_json_call_tool_result(serde_json::to_value(result).unwrap())
            }
            "move_file" => {
                let params: tool_impl::filesystem::MoveFileParamsMCP = serde_json::from_value(args_value.clone())
                    .map_err(|e| CallToolError::new(RpcError::new(RpcErrorCodes::INVALID_PARAMS, e.to_string(), None)))?;
                let result = tool_impl::filesystem::mcp_move_file(&self.deps, params).await.map_err(|e| mcp_call_tool_error_from_app_error(e, tool_name))?;
                create_mcp_json_call_tool_result(serde_json::to_value(result).unwrap())
            }
            "get_file_info" => {
                let params: tool_impl::filesystem::GetFileInfoParamsMCP = serde_json::from_value(args_value.clone())
                    .map_err(|e| CallToolError::new(RpcError::new(RpcErrorCodes::INVALID_PARAMS, e.to_string(), None)))?;
                let result = tool_impl::filesystem::mcp_get_file_info(&self.deps, params).await.map_err(|e| mcp_call_tool_error_from_app_error(e, tool_name))?;
                create_mcp_json_call_tool_result(serde_json::to_value(result).unwrap())
            }
            "read_multiple_files" => {
                let params: tool_impl::filesystem::ReadMultipleFilesParamsMCP = serde_json::from_value(args_value.clone())
                    .map_err(|e| CallToolError::new(RpcError::new(RpcErrorCodes::INVALID_PARAMS, e.to_string(), None)))?;
                let result = tool_impl::filesystem::mcp_read_multiple_files(&self.deps, params).await.map_err(|e| mcp_call_tool_error_from_app_error(e, tool_name))?;
                create_mcp_json_call_tool_result(serde_json::to_value(result).unwrap())
            }
            "search_files" => {
                let params: tool_impl::filesystem::SearchFilesParamsMCP = serde_json::from_value(args_value.clone())
                    .map_err(|e| CallToolError::new(RpcError::new(RpcErrorCodes::INVALID_PARAMS, e.to_string(), None)))?;
                let result = tool_impl::filesystem::mcp_search_files(&self.deps, params).await.map_err(|e| mcp_call_tool_error_from_app_error(e, tool_name))?;
                create_mcp_json_call_tool_result(serde_json::to_value(result).unwrap())
            }
            "search_code" => {
                let params: tool_impl::ripgrep::SearchCodeParamsMCP = serde_json::from_value(args_value.clone())
                    .map_err(|e| CallToolError::new(RpcError::new(RpcErrorCodes::INVALID_PARAMS, e.to_string(), None)))?;
                let result = tool_impl::ripgrep::mcp_search_code(&self.deps, params).await.map_err(|e| mcp_call_tool_error_from_app_error(e, tool_name))?;
                create_mcp_json_call_tool_result(serde_json::to_value(result).unwrap())
            }
            "execute_command" => {
                let params: tool_impl::terminal::ExecuteCommandParamsMCP = serde_json::from_value(args_value.clone())
                    .map_err(|e| CallToolError::new(RpcError::new(RpcErrorCodes::INVALID_PARAMS.into(), e.to_string(), None)))?;
                let result = tool_impl::terminal::mcp_execute_command(&self.deps, params).await.map_err(|e| mcp_call_tool_error_from_app_error(e, tool_name))?;
                create_mcp_json_call_tool_result(serde_json::to_value(result).unwrap())
            }
            "force_terminate_session" => {
                let params: tool_impl::terminal::ForceTerminateParamsMCP = serde_json::from_value(args_value.clone())
                    .map_err(|e| CallToolError::new(RpcError::new(RpcErrorCodes::INVALID_PARAMS.into(), e.to_string(), None)))?;
                let result = tool_impl::terminal::mcp_force_terminate_session(&self.deps, params).await.map_err(|e| mcp_call_tool_error_from_app_error(e, tool_name))?;
                create_mcp_json_call_tool_result(serde_json::to_value(result).unwrap())
            }
            "list_sessions" => {
                let result = tool_impl::terminal::mcp_list_sessions(&self.deps).await.map_err(|e| mcp_call_tool_error_from_app_error(e, tool_name))?;
                create_mcp_json_call_tool_result(serde_json::to_value(result).unwrap())
            }
            "read_session_output_status" => {
                let params: tool_impl::terminal::ReadOutputStatusParamsMCP = serde_json::from_value(args_value.clone())
                    .map_err(|e| CallToolError::new(RpcError::new(RpcErrorCodes::INVALID_PARAMS.into(), e.to_string(), None)))?;
                let result = tool_impl::terminal::mcp_read_session_output_status(&self.deps, params).await.map_err(|e| mcp_call_tool_error_from_app_error(e, tool_name))?;
                create_mcp_json_call_tool_result(serde_json::to_value(result).unwrap())
            }
            "list_processes" => {
                let result = tool_impl::process::mcp_list_processes(&self.deps).await.map_err(|e| mcp_call_tool_error_from_app_error(e, tool_name))?;
                create_mcp_json_call_tool_result(serde_json::to_value(result).unwrap())
            }
            "kill_process" => {
                let params: tool_impl::process::KillProcessParamsMCP = serde_json::from_value(args_value.clone())
                    .map_err(|e| CallToolError::new(RpcError::new(RpcErrorCodes::INVALID_PARAMS.into(), e.to_string(), None)))?;
                let result = tool_impl::process::mcp_kill_process(&self.deps, params).await.map_err(|e| mcp_call_tool_error_from_app_error(e, tool_name))?;
                create_mcp_json_call_tool_result(serde_json::to_value(result).unwrap())
            }
            "edit_block" => {
                let params: tool_impl::edit::EditBlockParamsMCP = serde_json::from_value(args_value.clone())
                    .map_err(|e| CallToolError::new(RpcError::new(RpcErrorCodes::INVALID_PARAMS.into(), e.to_string(), None)))?;
                let result = tool_impl::edit::mcp_edit_block(&self.deps, params).await.map_err(|e| mcp_call_tool_error_from_app_error(e, tool_name))?;
                create_mcp_json_call_tool_result(serde_json::to_value(result).unwrap())
            }
            _ => {
                error!("MCP: Unknown tool called: {}", tool_name);
                Err(CallToolError::unknown_tool(tool_name.to_string()))
            }
        }
    }
}