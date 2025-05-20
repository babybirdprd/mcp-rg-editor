use crate::config::Config;
use crate::error::AppError;
use crate::mcp::schemas::*;
use crate::tools::{
    edit_tool::{EditManager, EditBlockParams},
    filesystem_tool::{FilesystemManager, ReadFileParams, WriteFileParams, CreateDirectoryParams, ListDirectoryParams, MoveFileParams, SearchFilesParams, GetFileInfoParams},
    process_tool::{ProcessManager, KillProcessParams},
    ripgrep_tool::{RipgrepSearcher, SearchCodeParams},
    terminal_tool::{TerminalManager, ExecuteCommandParams, ReadOutputParams, ForceTerminateParams},
};

use async_trait::async_trait;
use rust_mcp_sdk::mcp_server::{ServerHandler, server_runtime::ServerRuntimeContext};
use rust_mcp_schema::{
    CallToolRequest, CallToolResult, ListToolsRequest, ListToolsResult, Tool,
    schema_utils::CallToolError,
};
use serde_json::Value;
use std::sync::Arc;
use tracing::{debug, error, info, instrument};

// Define a struct to hold all managers, initialized with the config
pub struct AppManagers {
    config: Arc<Config>, // Keep a clone of config for direct access if needed
    filesystem_manager: Arc<FilesystemManager>,
    terminal_manager: Arc<TerminalManager>,
    process_manager: Arc<ProcessManager>,
    ripgrep_searcher: Arc<RipgrepSearcher>,
    edit_manager: Arc<EditManager>,
}

impl AppManagers {
    pub fn new(config: Arc<Config>) -> Self {
        let filesystem_manager = Arc::new(FilesystemManager::new(config.clone()));
        let terminal_manager = Arc::new(TerminalManager::new(config.clone()));
        let process_manager = Arc::new(ProcessManager::new(config.clone()));
        let ripgrep_searcher = Arc::new(RipgrepSearcher::new(config.clone()));
        let edit_manager = Arc::new(EditManager::new(config.clone(), filesystem_manager.clone()));
        
        Self {
            config,
            filesystem_manager,
            terminal_manager,
            process_manager,
            ripgrep_searcher,
            edit_manager,
        }
    }
}


#[derive(Debug)]
pub struct EnhancedServerHandler {
   managers: Arc<AppManagers>,
}

impl EnhancedServerHandler {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            managers: Arc::new(AppManagers::new(config)),
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
            // Config
            Tool::new("get_config", "Get current server configuration.", get_config_schema()),
            Tool::new("set_config_value", "Set a server configuration value.", set_config_value_schema()),
            // Filesystem
            Tool::new("read_file", "Read content of a file.", read_file_schema()),
            Tool::new("write_file", "Write content to a file.", write_file_schema()),
            Tool::new("create_directory", "Create a directory.", create_directory_schema()),
            Tool::new("list_directory", "List contents of a directory.", list_directory_schema()),
            Tool::new("move_file", "Move or rename a file/directory.", move_file_schema()),
            Tool::new("search_files", "Search for files by name (substring match).", search_files_schema()),
            Tool::new("get_file_info", "Get metadata for a file or directory.", get_file_info_schema()),
            // Ripgrep Search
            Tool::new("search_code", "Search for text/code patterns in files using ripgrep.", search_code_schema()),
            // Edit
            Tool::new("edit_block", "Apply targeted text replacements to a file.", edit_block_schema()),
            // Terminal
            Tool::new("execute_command", "Execute a terminal command.", execute_command_schema()),
            Tool::new("read_output", "Read output from a running command session.", read_output_schema()),
            Tool::new("force_terminate", "Force terminate a command session.", force_terminate_schema()),
            Tool::new("list_sessions", "List active command sessions.", list_sessions_schema()),
            // Process
            Tool::new("list_processes", "List running processes on the system.", list_processes_schema()),
            Tool::new("kill_process", "Terminate a process by PID.", kill_process_schema()),
        ];
        Ok(ListToolsResult { tools, meta: None, next_cursor: None })
    }

    #[instrument(skip(self, request, _runtime), fields(tool_name = %request.params.name))]
    async fn handle_call_tool_request(
        &self,
        request: CallToolRequest,
        _runtime: &ServerRuntimeContext,
    ) -> Result<CallToolResult, CallToolError> {
        info!("Handling call_tool request for: {}", request.params.name);
        let args = request.params.arguments.unwrap_or_default();

        // Helper macro to parse args and call manager method
        macro_rules! handle_tool {
            ($manager_field:ident . $method:ident :: <$param_type:ty> ($($extra_args:expr),*)) => {
                {
                    let params: $param_type = serde_json::from_value(Value::Object(args))
                        .map_err(|e| CallToolError::invalid_params(e.to_string()))?;
                    let result = self.managers.$manager_field.$method($($extra_args,)* ¶ms).await?;
                    Ok(CallToolResult::from_serializable(&result)?)
                }
            };
             ($manager_field:ident . $method:ident ()) => {
                {
                    let result = self.managers.$manager_field.$method().await?;
                    Ok(CallToolResult::from_serializable(&result)?)
                }
            };
        }
        
        // Helper for config tools (they are not in AppManagers directly)
        macro_rules! handle_config_tool {
            (get_config) => {
                {
                    let config_clone = self.managers.config.as_ref().clone(); // Clone the config data
                    Ok(CallToolResult::from_serializable(&config_clone)?)
                }
            };
            (set_config_value) => {
                Err(CallToolError::internal_error("set_config_value is not implemented yet. Configuration is via environment variables.".to_string()))
                // TODO: Implement mutable config if desired. Requires RwLock for Config.
            };
        }


        match request.params.name.as_str() {
            // Config
            "get_config" => handle_config_tool!(get_config),
            "set_config_value" => handle_config_tool!(set_config_value),
            // Filesystem
            "read_file" => handle_tool!(filesystem_manager.read_file:: <ReadFileParams>()),
            "write_file" => handle_tool!(filesystem_manager.write_file:: <WriteFileParams>()),
            "create_directory" => handle_tool!(filesystem_manager.create_directory:: <CreateDirectoryParams>()),
            "list_directory" => handle_tool!(filesystem_manager.list_directory:: <ListDirectoryParams>()),
            "move_file" => handle_tool!(filesystem_manager.move_file:: <MoveFileParams>()),
            "search_files" => handle_tool!(filesystem_manager.search_files:: <SearchFilesParams>()),
            "get_file_info" => handle_tool!(filesystem_manager.get_file_info:: <GetFileInfoParams>()),
            // Ripgrep
            "search_code" => handle_tool!(ripgrep_searcher.search_code:: <SearchCodeParams>()),
            // Edit
            "edit_block" => handle_tool!(edit_manager.edit_block:: <EditBlockParams>()),
            // Terminal
            "execute_command" => handle_tool!(terminal_manager.execute_command:: <ExecuteCommandParams>()),
            "read_output" => handle_tool!(terminal_manager.read_output:: <ReadOutputParams>()),
            "force_terminate" => handle_tool!(terminal_manager.force_terminate:: <ForceTerminateParams>()),
            "list_sessions" => handle_tool!(terminal_manager.list_sessions()),
            // Process
            "list_processes" => handle_tool!(process_manager.list_processes()),
            "kill_process" => handle_tool!(process_manager.kill_process:: <KillProcessParams>()),

            _ => {
                error!("Unknown tool called: {}", request.params.name);
                Err(CallToolError::unknown_tool(request.params.name))
            }
        }
    }
}