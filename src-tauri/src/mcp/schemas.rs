// FILE: src-tauri/src/mcp/schemas.rs
// IMPORTANT NOTE: Rewrite the entire file.
// These schemas are for the MCP interface.
use rust_mcp_schema::ToolInputSchema;
use serde_json::{json, Value};
use std::collections::HashMap;

fn create_prop(type_str: &str, description: &str) -> Value {
    json!({ "type": type_str, "description": description })
}
fn create_prop_with_default_bool(type_str: &str, description: &str, default_val: bool) -> Value {
    json!({ "type": type_str, "description": description, "default": default_val })
}
fn create_prop_with_default_int(type_str: &str, description: &str, default_val: usize) -> Value {
    json!({ "type": type_str, "description": description, "default": default_val })
}
fn create_prop_with_default_str(type_str: &str, description: &str, default_val: &str) -> Value {
    json!({ "type": type_str, "description": description, "default": default_val })
}
fn create_array_prop(item_type_str: &str, description: &str) -> Value {
    json!({ "type": "array", "items": { "type": item_type_str }, "description": description })
}
fn create_enum_prop(enum_values: Vec<&str>, default_value: &str, description: &str) -> Value {
    json!({ "type": "string", "enum": enum_values, "default": default_value, "description": description })
}

const MCP_PATH_GUIDANCE: &str = "IMPORTANT: Paths should be absolute or tilde-expanded (~/...). The server will resolve them against its configured FILES_ROOT if relative, but absolute/tilde is preferred for clarity.";

fn build_mcp_schema(required: Vec<String>, properties: Option<HashMap<String, Value>>) -> Option<Value> {
    properties.map(|props| {
        json!({ "type": "object", "properties": props, "required": required })
    })
}

// --- MCP Tool Schemas ---
pub fn get_mcp_config_schema() -> ToolInputSchema {
    ToolInputSchema::new(vec![], None) // No input params for getting config
}

pub fn read_file_mcp_schema() -> ToolInputSchema {
    let mut props = HashMap::new();
    props.insert("path".to_string(), create_prop("string", &format!("Path to the file or URL. {}", MCP_PATH_GUIDANCE)));
    props.insert("is_url".to_string(), create_prop_with_default_bool("boolean", "True if 'path' is a URL.", false));
    props.insert("offset".to_string(), create_prop_with_default_int("integer", "Line offset for text files.", 0));
    props.insert("length".to_string(), json!({"type": "integer", "description": "Max lines to read for text files."}));
    let req = vec!["path".to_string()];
    ToolInputSchema::new(req.clone(), build_mcp_schema(req, Some(props)))
}

pub fn write_file_mcp_schema() -> ToolInputSchema {
    let mut props = HashMap::new();
    props.insert("path".to_string(), create_prop("string", &format!("File path. {}", MCP_PATH_GUIDANCE)));
    props.insert("content".to_string(), create_prop("string", "Content to write."));
    props.insert("mode".to_string(), create_enum_prop(vec!["rewrite", "append"], "rewrite", "Write mode."));
    let req = vec!["path".to_string(), "content".to_string()];
    ToolInputSchema::new(req.clone(), build_mcp_schema(req, Some(props)))
}

pub fn create_directory_mcp_schema() -> ToolInputSchema {
    let mut props = HashMap::new();
    props.insert("path".to_string(), create_prop("string", &format!("Directory path to create. {}", MCP_PATH_GUIDANCE)));
    let req = vec!["path".to_string()];
    ToolInputSchema::new(req.clone(), build_mcp_schema(req, Some(props)))
}

pub fn list_directory_mcp_schema() -> ToolInputSchema {
    let mut props = HashMap::new();
    props.insert("path".to_string(), create_prop("string", &format!("Directory path to list. {}", MCP_PATH_GUIDANCE)));
    let req = vec!["path".to_string()];
    ToolInputSchema::new(req.clone(), build_mcp_schema(req, Some(props)))
}

pub fn move_file_mcp_schema() -> ToolInputSchema {
    let mut props = HashMap::new();
    props.insert("source".to_string(), create_prop("string", &format!("Source path. {}", MCP_PATH_GUIDANCE)));
    props.insert("destination".to_string(), create_prop("string", &format!("Destination path. {}", MCP_PATH_GUIDANCE)));
    let req = vec!["source".to_string(), "destination".to_string()];
    ToolInputSchema::new(req.clone(), build_mcp_schema(req, Some(props)))
}

pub fn get_file_info_mcp_schema() -> ToolInputSchema {
    let mut props = HashMap::new();
    props.insert("path".to_string(), create_prop("string", &format!("File/directory path. {}", MCP_PATH_GUIDANCE)));
    let req = vec!["path".to_string()];
    ToolInputSchema::new(req.clone(), build_mcp_schema(req, Some(props)))
}

pub fn read_multiple_files_mcp_schema() -> ToolInputSchema {
    let mut props = HashMap::new();
    props.insert("paths".to_string(), create_array_prop("string", &format!("Array of file paths. {}", MCP_PATH_GUIDANCE)));
    let req = vec!["paths".to_string()];
    ToolInputSchema::new(req.clone(), build_mcp_schema(req, Some(props)))
}

pub fn search_files_mcp_schema() -> ToolInputSchema {
    let mut props = HashMap::new();
    props.insert("path".to_string(), create_prop("string", &format!("Root path for search. {}", MCP_PATH_GUIDANCE)));
    props.insert("pattern".to_string(), create_prop("string", "Search pattern for file/dir names."));
    props.insert("timeoutMs".to_string(), json!({"type": "integer", "description": "Timeout in ms."}));
    props.insert("recursive".to_string(), create_prop_with_default_bool("boolean", "Search recursively.", true));
    props.insert("max_depth".to_string(), create_prop_with_default_int("integer", "Max recursion depth.", 10));
    let req = vec!["path".to_string(), "pattern".to_string()];
    ToolInputSchema::new(req.clone(), build_mcp_schema(req, Some(props)))
}


pub fn search_code_mcp_schema() -> ToolInputSchema {
    let mut props = HashMap::new();
    props.insert("pattern".to_string(), create_prop("string", "Ripgrep search pattern."));
    props.insert("path".to_string(), create_prop_with_default_str("string", &format!("Directory to search. Default: FILES_ROOT. {}", MCP_PATH_GUIDANCE), "."));
    props.insert("fixed_strings".to_string(), create_prop_with_default_bool("boolean", "Treat pattern as literal.", false));
    props.insert("ignore_case".to_string(), create_prop_with_default_bool("boolean", "Case-insensitive search.", false));
    props.insert("case_sensitive".to_string(), create_prop_with_default_bool("boolean", "Case-sensitive search.", false));
    props.insert("line_numbers".to_string(), create_prop_with_default_bool("boolean", "Include line numbers.", true));
    props.insert("context_lines".to_string(), create_prop_with_default_int("integer", "Context lines around match.", 0));
    props.insert("file_pattern".to_string(), json!({"type": "string", "description": "Glob to filter files (e.g., \"*.rs\")."}));
    props.insert("max_depth".to_string(), json!({"type": "integer", "description": "Max search depth."}));
    props.insert("max_results".to_string(), create_prop_with_default_int("integer", "Max matches to return.", 1000));
    props.insert("include_hidden".to_string(), create_prop_with_default_bool("boolean", "Search hidden files/dirs.", false));
    props.insert("timeoutMs".to_string(), json!({"type": "integer", "description": "Timeout in ms."}));
    let req = vec!["pattern".to_string()];
    ToolInputSchema::new(req.clone(), build_mcp_schema(req, Some(props)))
}

pub fn execute_command_mcp_schema() -> ToolInputSchema {
    let mut props = HashMap::new();
    props.insert("command".to_string(), create_prop("string", "Command to execute."));
    props.insert("timeout_ms".to_string(), create_prop_with_default_int("integer", "Timeout for initial output (ms).", 1000));
    props.insert("shell".to_string(), json!({"type": "string", "description": "Specific shell (e.g., bash, powershell)."}));
    let req = vec!["command".to_string()];
    ToolInputSchema::new(req.clone(), build_mcp_schema(req, Some(props)))
}

pub fn force_terminate_mcp_schema() -> ToolInputSchema {
    let mut props = HashMap::new();
    props.insert("session_id".to_string(), create_prop("string", "ID of command session to terminate."));
    let req = vec!["session_id".to_string()];
    ToolInputSchema::new(req.clone(), build_mcp_schema(req, Some(props)))
}

pub fn list_sessions_mcp_schema() -> ToolInputSchema {
    ToolInputSchema::new(vec![], None)
}

pub fn read_session_output_status_mcp_schema() -> ToolInputSchema {
    let mut props = HashMap::new();
    props.insert("session_id".to_string(), create_prop("string", "ID of command session."));
    let req = vec!["session_id".to_string()];
    ToolInputSchema::new(req.clone(), build_mcp_schema(req, Some(props)))
}

pub fn list_processes_mcp_schema() -> ToolInputSchema {
    ToolInputSchema::new(vec![], None)
}

pub fn kill_process_mcp_schema() -> ToolInputSchema {
    let mut props = HashMap::new();
    props.insert("pid".to_string(), create_prop("integer", "Process ID (PID) to terminate."));
    let req = vec!["pid".to_string()];
    ToolInputSchema::new(req.clone(), build_mcp_schema(req, Some(props)))
}

pub fn edit_block_mcp_schema() -> ToolInputSchema {
    let mut props = HashMap::new();
    props.insert("file_path".to_string(), create_prop("string", &format!("File path. {}", MCP_PATH_GUIDANCE)));
    props.insert("old_string".to_string(), create_prop("string", "Exact string to replace."));
    props.insert("new_string".to_string(), create_prop("string", "String to replace with."));
    props.insert("expected_replacements".to_string(), create_prop_with_default_int("integer", "Expected number of replacements (0 for all).", 1));
    let req = vec!["file_path".to_string(), "old_string".to_string(), "new_string".to_string()];
    ToolInputSchema::new(req.clone(), build_mcp_schema(req, Some(props)))
}