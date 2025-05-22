// FILE: src-tauri/src/mcp/handler.rs
// IMPORTANT NOTE: Rewrite the entire file.
use crate::config::Config;
use crate::error::AppError; // Your main AppError
use crate::mcp::schemas::*; // MCP tool schemas you'll define
use crate::mcp::tool_impl; // Where actual tool logic will reside
use crate::utils::audit_logger::AuditLogger as AppAuditLogger; // Alias to avoid confusion
use crate::utils::fuzzy_search_logger::FuzzySearchLogger as AppFuzzySearchLogger;
use crate::commands::terminal_commands::ActiveSessionsMap; // For terminal state
use sysinfo::System as SysinfoSystem; // For process state


use async_trait::async_trait;
use rust_mcp_sdk::mcp_server::{ServerHandler, server_runtime::ServerRuntimeContext};
use rust_mcp_schema::{
    CallToolRequest, CallToolResult, ListToolsRequest, ListToolsResult, Tool, Content,
    schema_utils::CallToolError, RpcError, RpcErrorCode,
};
use serde_json::{Value, json};
use std::sync::{Arc, RwLock as StdRwLock};
use tauri::{AppHandle, Manager, State as TauriState}; // TauriState for AppHandle managed state
use tokio::sync::Mutex as TokioMutex; // For SysinfoSystem
use tracing::{debug, error, info, instrument, warn};

// This struct will hold dependencies needed by the tool implementations
#[derive(Clone)]
pub struct ToolDependencies {
    pub app_handle: AppHandle,
    pub config_state: Arc<StdRwLock<Config>>,
    pub audit_logger: Arc<AppAuditLogger>,
    pub fuzzy_search_logger: Arc<AppFuzzySearchLogger>,
    pub active_sessions_map: ActiveSessionsMap, // Arc<TokioMutex<HashMap<String, Arc<ActiveSession>>>>
    pub sysinfo_state: Arc<TokioMutex<SysinfoSystem>>,
}

#[derive(Clone)]
pub struct EnhancedServerHandler {
   deps: ToolDependencies,
}

impl EnhancedServerHandler {
    pub fn new(app_handle: AppHandle, config_state: Arc<StdRwLock<Config>>) -> Self {
        // Retrieve managed states from AppHandle
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
    let rpc_error_code = match app_err {
        AppError::InvalidInputArgument(_) | AppError::PathNotAllowed(_) | AppError::PathTraversal(_) | AppError::InvalidPath(_) => RpcErrorCode::InvalidParams,
        AppError::CommandBlocked(_) => RpcErrorCode::ServerError(-32001),
        _ => RpcErrorCode::InternalError,
    };
    CallToolError::new(RpcError::new(rpc_error_code, app_err.to_string(), None))
}

fn create_mcp_json_call_tool_result(value: Value) -> Result<CallToolResult, CallToolError> {
    let content_item = Content::Other {
        type_: "json".to_string(),
        data: Some(value),
        text: None, mime_type: Some("application/json".to_string()), resource_id: None,
        name: None, size: None, created_at: None, updated_at: None, meta: None,
    };
    Ok(CallToolResult::new(vec![content_item], None))
}


#[async_trait]
impl ServerHandler for EnhancedServerHandler {
    #[instrument(skip(self, _request, _runtime_context))]
    async fn handle_list_tools_request(
        &self,
        _request: ListToolsRequest,
        _runtime_context: &ServerRuntimeContext,
    ) -> Result<ListToolsResult, RpcError> {
        info!("MCP: Handling list_tools request");
        let tools = vec![
            Tool::new("mcp_get_config".to_string(), Some("Get the MCP server's current runtime configuration.".to_string()), Some(get_mcp_config_schema()), None),
            Tool::new("read_file".to_string(), Some("Read content of a local file or URL.".to_string()), Some(read_file_mcp_schema()), None),
            Tool::new("write_file".to_string(), Some("Write/append content to a file.".to_string()), Some(write_file_mcp_schema()), None),
            Tool::new("create_directory".to_string(), Some("Create directories, including nested ones.".to_string()), Some(create_directory_mcp_schema()), None),
            Tool::new("list_directory".to_string(), Some("List directory contents.".to_string()), Some(list_directory_mcp_schema()), None),
            Tool::new("move_file".to_string(), Some("Move or rename files or directories.".to_string()), Some(move_file_mcp_schema()), None),
            Tool::new("get_file_info".to_string(), Some("Get metadata for a file or directory.".to_string()), Some(get_file_info_mcp_schema()), None),
            Tool::new("read_multiple_files".to_string(), Some("Read multiple local files.".to_string()), Some(read_multiple_files_mcp_schema()), None),
            Tool::new("search_files".to_string(), Some("Find files/dirs by name.".to_string()), Some(search_files_mcp_schema()), None),
            Tool::new("search_code".to_string(), Some("Search code with Ripgrep.".to_string()), Some(search_code_mcp_schema()), None),
            Tool::new("execute_command".to_string(), Some("Run terminal commands. Output is streamed via events if using Tauri UI; for MCP, initial output/status returned.".to_string()), Some(execute_command_mcp_schema()), None),
            Tool::new("force_terminate_session".to_string(), Some("Stop a running command session by its ID.".to_string()), Some(force_terminate_mcp_schema()), None),
            Tool::new("list_sessions".to_string(), Some("List active command sessions.".to_string()), Some(list_sessions_mcp_schema()), None),
            Tool::new("read_session_output_status".to_string(), Some("Get status of a command session. For MCP, this might include buffered output if designed so.".to_string()), Some(read_session_output_status_mcp_schema()), None),
            Tool::new("list_processes".to_string(), Some("List system processes.".to_string()), Some(list_processes_mcp_schema()), None),
            Tool::new("kill_process".to_string(), Some("Terminate a system process by PID.".to_string()), Some(kill_process_mcp_schema()), None),
            Tool::new("edit_block".to_string(), Some("Apply targeted text replacements in a file.".to_string()), Some(edit_block_mcp_schema()), None),
        ];
        Ok(ListToolsResult { tools, meta: None, next_cursor: None })
    }

    #[instrument(skip(self, request, _runtime_context), fields(tool_name = %request.params.name))]
    async fn handle_call_tool_request(
        &self,
        request: CallToolRequest,
        _runtime_context: &ServerRuntimeContext,
    ) -> Result<CallToolResult, CallToolError> {
        let tool_name = request.params.name.as_str();
        let args_value = request.params.arguments.clone().unwrap_or_else(|| json!({}));
        info!(tool_name = %tool_name, "MCP: Handling call_tool request");
        
        // Audit logging for MCP tool calls
        self.deps.audit_logger.log_command_call(tool_name, &args_value).await;


        match tool_name {
            "mcp_get_config" => {
                let config_guard = self.deps.config_state.read().map_err(|e| CallToolError::new(RpcError::internal_error(Some(format!("Config lock error: {}", e)))))?;
                let current_config_data = config_guard.clone();
                drop(config_guard);
                let value_result = serde_json::to_value(current_config_data)
                    .map_err(|e| CallToolError::new(RpcError::internal_error(Some(format!("Failed to serialize config: {}", e)))))?;
                create_mcp_json_call_tool_result(value_result)
            }
            // Filesystem Tools
            "read_file" => {
                let params: tool_impl::filesystem::ReadFileParamsMCP = serde_json::from_value(args_value).map_err(|e| CallToolError::invalid_params(e.to_string()))?;
                let result = tool_impl::filesystem::mcp_read_file(&self.deps, params).await.map_err(|e| mcp_call_tool_error_from_app_error(e, tool_name))?;
                create_mcp_json_call_tool_result(serde_json::to_value(result).unwrap())
            }
            "write_file" => {
                let params: tool_impl::filesystem::WriteFileParamsMCP = serde_json::from_value(args_value).map_err(|e| CallToolError::invalid_params(e.to_string()))?;
                let result = tool_impl::filesystem::mcp_write_file(&self.deps, params).await.map_err(|e| mcp_call_tool_error_from_app_error(e, tool_name))?;
                create_mcp_json_call_tool_result(serde_json::to_value(result).unwrap())
            }
             "create_directory" => {
                let params: tool_impl::filesystem::CreateDirectoryParamsMCP = serde_json::from_value(args_value).map_err(|e| CallToolError::invalid_params(e.to_string()))?;
                let result = tool_impl::filesystem::mcp_create_directory(&self.deps, params).await.map_err(|e| mcp_call_tool_error_from_app_error(e, tool_name))?;
                create_mcp_json_call_tool_result(serde_json::to_value(result).unwrap())
            }
            "list_directory" => {
                let params: tool_impl::filesystem::ListDirectoryParamsMCP = serde_json::from_value(args_value).map_err(|e| CallToolError::invalid_params(e.to_string()))?;
                let result = tool_impl::filesystem::mcp_list_directory(&self.deps, params).await.map_err(|e| mcp_call_tool_error_from_app_error(e, tool_name))?;
                create_mcp_json_call_tool_result(serde_json::to_value(result).unwrap())
            }
            "move_file" => {
                let params: tool_impl::filesystem::MoveFileParamsMCP = serde_json::from_value(args_value).map_err(|e| CallToolError::invalid_params(e.to_string()))?;
                let result = tool_impl::filesystem::mcp_move_file(&self.deps, params).await.map_err(|e| mcp_call_tool_error_from_app_error(e, tool_name))?;
                create_mcp_json_call_tool_result(serde_json::to_value(result).unwrap())
            }
            "get_file_info" => {
                let params: tool_impl::filesystem::GetFileInfoParamsMCP = serde_json::from_value(args_value).map_err(|e| CallToolError::invalid_params(e.to_string()))?;
                let result = tool_impl::filesystem::mcp_get_file_info(&self.deps, params).await.map_err(|e| mcp_call_tool_error_from_app_error(e, tool_name))?;
                create_mcp_json_call_tool_result(serde_json::to_value(result).unwrap())
            }
            "read_multiple_files" => {
                let params: tool_impl::filesystem::ReadMultipleFilesParamsMCP = serde_json::from_value(args_value).map_err(|e| CallToolError::invalid_params(e.to_string()))?;
                let result = tool_impl::filesystem::mcp_read_multiple_files(&self.deps, params).await.map_err(|e| mcp_call_tool_error_from_app_error(e, tool_name))?;
                create_mcp_json_call_tool_result(serde_json::to_value(result).unwrap())
            }
            "search_files" => {
                let params: tool_impl::filesystem::SearchFilesParamsMCP = serde_json::from_value(args_value).map_err(|e| CallToolError::invalid_params(e.to_string()))?;
                let result = tool_impl::filesystem::mcp_search_files(&self.deps, params).await.map_err(|e| mcp_call_tool_error_from_app_error(e, tool_name))?;
                create_mcp_json_call_tool_result(serde_json::to_value(result).unwrap())
            }
            // Ripgrep Tool
            "search_code" => {
                let params: tool_impl::ripgrep::SearchCodeParamsMCP = serde_json::from_value(args_value).map_err(|e| CallToolError::invalid_params(e.to_string()))?;
                let result = tool_impl::ripgrep::mcp_search_code(&self.deps, params).await.map_err(|e| mcp_call_tool_error_from_app_error(e, tool_name))?;
                create_mcp_json_call_tool_result(serde_json::to_value(result).unwrap())
            }
            // Terminal Tools
            "execute_command" => {
                let params: tool_impl::terminal::ExecuteCommandParamsMCP = serde_json::from_value(args_value).map_err(|e| CallToolError::invalid_params(e.to_string()))?;
                let result = tool_impl::terminal::mcp_execute_command(&self.deps, params).await.map_err(|e| mcp_call_tool_error_from_app_error(e, tool_name))?;
                create_mcp_json_call_tool_result(serde_json::to_value(result).unwrap())
            }
            "force_terminate_session" => {
                let params: tool_impl::terminal::ForceTerminateParamsMCP = serde_json::from_value(args_value).map_err(|e| CallToolError::invalid_params(e.to_string()))?;
                let result = tool_impl::terminal::mcp_force_terminate_session(&self.deps, params).await.map_err(|e| mcp_call_tool_error_from_app_error(e, tool_name))?;
                create_mcp_json_call_tool_result(serde_json::to_value(result).unwrap())
            }
            "list_sessions" => {
                let result = tool_impl::terminal::mcp_list_sessions(&self.deps).await.map_err(|e| mcp_call_tool_error_from_app_error(e, tool_name))?;
                create_mcp_json_call_tool_result(serde_json::to_value(result).unwrap())
            }
            "read_session_output_status" => {
                let params: tool_impl::terminal::ReadOutputStatusParamsMCP = serde_json::from_value(args_value).map_err(|e| CallToolError::invalid_params(e.to_string()))?;
                let result = tool_impl::terminal::mcp_read_session_output_status(&self.deps, params).await.map_err(|e| mcp_call_tool_error_from_app_error(e, tool_name))?;
                create_mcp_json_call_tool_result(serde_json::to_value(result).unwrap())
            }
            // Process Tools
            "list_processes" => {
                let result = tool_impl::process::mcp_list_processes(&self.deps).await.map_err(|e| mcp_call_tool_error_from_app_error(e, tool_name))?;
                create_mcp_json_call_tool_result(serde_json::to_value(result).unwrap())
            }
            "kill_process" => {
                let params: tool_impl::process::KillProcessParamsMCP = serde_json::from_value(args_value).map_err(|e| CallToolError::invalid_params(e.to_string()))?;
                let result = tool_impl::process::mcp_kill_process(&self.deps, params).await.map_err(|e| mcp_call_tool_error_from_app_error(e, tool_name))?;
                create_mcp_json_call_tool_result(serde_json::to_value(result).unwrap())
            }
            // Edit Tool
            "edit_block" => {
                let params: tool_impl::edit::EditBlockParamsMCP = serde_json::from_value(args_value).map_err(|e| CallToolError::invalid_params(e.to_string()))?;
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