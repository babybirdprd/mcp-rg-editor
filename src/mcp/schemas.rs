use rust_mcp_schema::ToolInputSchema;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

// Helper to create a JSON schema property
fn create_prop(type_str: &str, description: &str) -> Map<String, Value> {
    let mut prop = Map::new();
    prop.insert("type".to_string(), json!(type_str));
    prop.insert("description".to_string(), json!(description));
    prop
}

fn create_array_prop(item_type_str: &str, description: &str) -> Map<String, Value> {
    let mut items_prop = Map::new();
    items_prop.insert("type".to_string(), json!(item_type_str));
    
    let mut prop = Map::new();
    prop.insert("type".to_string(), json!("array"));
    prop.insert("items".to_string(), Value::Object(items_prop));
    prop.insert("description".to_string(), json!(description));
    prop
}

fn create_enum_prop(enum_values: Vec<&str>, default_value: &str, description: &str) -> Map<String, Value> {
    let mut prop = Map::new();
    prop.insert("type".to_string(), json!("string"));
    prop.insert("enum".to_string(), json!(enum_values));
    prop.insert("default".to_string(), json!(default_value));
    prop.insert("description".to_string(), json!(description));
    prop
}


pub fn get_config_schema() -> ToolInputSchema {
    ToolInputSchema::new(vec![], None)
}

pub fn set_config_value_schema() -> ToolInputSchema {
    let mut properties = HashMap::new();
    properties.insert("key".to_string(), create_prop("string", "Configuration key to set."));
    properties.insert("value".to_string(), json!({
        "description": "Value to set for the key. Can be string, number, boolean, or array (as JSON string).",
        "oneOf": [
            { "type": "string" },
            { "type": "number" },
            { "type": "boolean" },
            { "type": "array", "items": { "type": "string" } }
        ]
    }));
    ToolInputSchema::new(vec!["key".to_string(), "value".to_string()], Some(properties))
}

pub fn read_file_schema() -> ToolInputSchema {
    let mut properties = HashMap::new();
    properties.insert("path".to_string(), create_prop("string", "Absolute path to the file or URL."));
    properties.insert("offset".to_string(), json!({
        "type": "integer",
        "description": "Line number to start reading from (0-indexed). Default 0.",
        "default": 0
    }));
     properties.insert("length".to_string(), json!({
        "type": "integer",
        "description": "Maximum number of lines to read. Uses config default if not set.",
        "optional": true
    }));
    ToolInputSchema::new(vec!["path".to_string()], Some(properties))
}

pub fn write_file_schema() -> ToolInputSchema {
    let mut properties = HashMap::new();
    properties.insert("path".to_string(), create_prop("string", "Absolute path to the file."));
    properties.insert("content".to_string(), create_prop("string", "Content to write."));
    properties.insert("mode".to_string(), create_enum_prop(vec!["rewrite", "append"], "rewrite", "Write mode."));
    ToolInputSchema::new(vec!["path".to_string(), "content".to_string()], Some(properties))
}

pub fn create_directory_schema() -> ToolInputSchema {
    let mut properties = HashMap::new();
    properties.insert("path".to_string(), create_prop("string", "Absolute path of the directory to create."));
    ToolInputSchema::new(vec!["path".to_string()], Some(properties))
}

pub fn list_directory_schema() -> ToolInputSchema {
    let mut properties = HashMap::new();
    properties.insert("path".to_string(), create_prop("string", "Absolute path of the directory to list."));
    ToolInputSchema::new(vec!["path".to_string()], Some(properties))
}

pub fn move_file_schema() -> ToolInputSchema {
    let mut properties = HashMap::new();
    properties.insert("source".to_string(), create_prop("string", "Absolute source path."));
    properties.insert("destination".to_string(), create_prop("string", "Absolute destination path."));
    ToolInputSchema::new(vec!["source".to_string(), "destination".to_string()], Some(properties))
}

pub fn search_files_schema() -> ToolInputSchema { // Simple name search
    let mut properties = HashMap::new();
    properties.insert("path".to_string(), create_prop("string", "Absolute root path for search."));
    properties.insert("pattern".to_string(), create_prop("string", "Substring to search for in file/directory names."));
    ToolInputSchema::new(vec!["path".to_string(), "pattern".to_string()], Some(properties))
}

pub fn get_file_info_schema() -> ToolInputSchema {
    let mut properties = HashMap::new();
    properties.insert("path".to_string(), create_prop("string", "Absolute path to the file or directory."));
    ToolInputSchema::new(vec!["path".to_string()], Some(properties))
}

pub fn search_code_schema() -> ToolInputSchema { // Ripgrep
    let mut properties = HashMap::new();
    properties.insert("pattern".to_string(), create_prop("string", "Search pattern (regex or literal)."));
    properties.insert("path".to_string(), create_prop("string", "Directory to search within (relative to FILES_ROOT, or absolute if allowed). Default is FILES_ROOT."));
    properties.insert("fixed_strings".to_string(), create_prop("boolean", "Treat pattern as literal string. Default false."));
    properties.insert("case_sensitive".to_string(), create_prop("boolean", "Perform case-sensitive search. Default false (ripgrep default is smart case)."));
    properties.insert("line_numbers".to_string(), create_prop("boolean", "Include line numbers. Default true."));
    properties.insert("context_lines".to_string(), create_prop("integer", "Number of context lines around matches. Default 0."));
    properties.insert("file_types".to_string(), create_array_prop("string", "List of file types (e.g., 'rust', 'py'). See 'rg --type-list'."));
    properties.insert("max_depth".to_string(), create_prop("integer", "Maximum search depth."));
    properties.insert("max_results".to_string(), json!({
        "type": "integer",
        "description": "Maximum number of matches to return. Default 1000.",
        "default": 1000
    }));
    ToolInputSchema::new(vec!["pattern".to_string()], Some(properties))
}

pub fn edit_block_schema() -> ToolInputSchema {
    let mut properties = HashMap::new();
    properties.insert("file_path".to_string(), create_prop("string", "Absolute path to the file."));
    properties.insert("old_string".to_string(), create_prop("string", "The exact string to be replaced."));
    properties.insert("new_string".to_string(), create_prop("string", "The string to replace with."));
    properties.insert("expected_replacements".to_string(), json!({
        "type": "integer",
        "description": "Number of occurrences expected to be replaced. If 0, replaces all occurrences. Default 1.",
        "default": 1
    }));
    ToolInputSchema::new(
        vec!["file_path".to_string(), "old_string".to_string(), "new_string".to_string()],
        Some(properties),
    )
}

pub fn execute_command_schema() -> ToolInputSchema {
    let mut properties = HashMap::new();
    properties.insert("command".to_string(), create_prop("string", "The command to execute."));
    properties.insert("timeout_ms".to_string(), create_prop("integer", "Timeout in milliseconds for initial output. Default 1000."));
    properties.insert("shell".to_string(), create_prop("string", "Specific shell to use (e.g., 'bash', 'powershell'). Uses config default if not set."));
    ToolInputSchema::new(vec!["command".to_string()], Some(properties))
}

pub fn read_output_schema() -> ToolInputSchema {
    let mut properties = HashMap::new();
    properties.insert("session_id".to_string(), create_prop("string", "ID of the command session."));
    ToolInputSchema::new(vec!["session_id".to_string()], Some(properties))
}

pub fn force_terminate_schema() -> ToolInputSchema {
    let mut properties = HashMap::new();
    properties.insert("session_id".to_string(), create_prop("string", "ID of the command session to terminate."));
    ToolInputSchema::new(vec!["session_id".to_string()], Some(properties))
}

pub fn list_sessions_schema() -> ToolInputSchema {
    ToolInputSchema::new(vec![], None)
}

pub fn list_processes_schema() -> ToolInputSchema {
    ToolInputSchema::new(vec![], None)
}

pub fn kill_process_schema() -> ToolInputSchema {
    let mut properties = HashMap::new();
    properties.insert("pid".to_string(), create_prop("integer", "Process ID (PID) to terminate."));
    ToolInputSchema::new(vec!["pid".to_string()], Some(properties))
}