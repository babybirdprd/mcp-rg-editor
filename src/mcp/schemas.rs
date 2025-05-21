use rust_mcp_schema::ToolInputSchema;
use serde_json::{json, Map, Value}; // Value is fine, Map not needed here for properties
use std::collections::HashMap;

// Helper to create a JSON schema property
fn create_prop(type_str: &str, description: &str) -> Value {
    json!({
        "type": type_str,
        "description": description
    })
}

fn create_prop_with_default_str(type_str: &str, description: &str, default_val: &str) -> Value {
    json!({
        "type": type_str,
        "description": description,
        "default": default_val
    })
}

fn create_prop_with_default_bool(type_str: &str, description: &str, default_val: bool) -> Value {
    json!({
        "type": type_str,
        "description": description,
        "default": default_val
    })
}

fn create_prop_with_default_int(type_str: &str, description: &str, default_val: usize) -> Value {
    json!({
        "type": type_str,
        "description": description,
        "default": default_val
    })
}

fn create_array_prop(item_type_str: &str, description: &str) -> Value {
    json!({
        "type": "array",
        "items": { "type": item_type_str },
        "description": description
    })
}

fn create_enum_prop(enum_values: Vec<&str>, default_value: &str, description: &str) -> Value {
    json!({
        "type": "string",
        "enum": enum_values,
        "default": default_value,
        "description": description
    })
}

const PATH_GUIDANCE: &str = "IMPORTANT: Always use absolute paths (starting with '/' or drive letter like 'C:\\') or tilde-expanded paths (~/...). Relative paths are resolved against FILES_ROOT.";
// const CMD_PREFIX_DESCRIPTION: &str = "This command can be referenced as \"DC: ...\" or \"use Desktop Commander to ...\" in your instructions.";


pub fn get_config_schema() -> ToolInputSchema {
    ToolInputSchema::new(vec![], None)
}

pub fn set_config_value_schema() -> ToolInputSchema {
    let mut properties = HashMap::new();
    properties.insert("key".to_string(), create_prop("string", "Configuration key to set. Valid keys: blockedCommands, defaultShell, allowedDirectories, fileReadLineLimit, fileWriteLineLimit."));
    properties.insert("value".to_string(), json!({
        "description": "Value to set. For array keys (blockedCommands, allowedDirectories), provide a JSON array of strings. For others, a simple string or number.",
        "anyOf": [
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
    properties.insert("path".to_string(), create_prop("string", &format!("Path to the file or URL. {}", PATH_GUIDANCE)));
    properties.insert("is_url".to_string(), create_prop_with_default_bool("boolean", "Set to true if 'path' is a URL. Default false.", false));
    properties.insert("offset".to_string(), create_prop_with_default_int("integer", "Line number to start reading from (0-indexed) for local text files. Default 0.", 0));
    properties.insert("length".to_string(), json!({
        "type": "integer",
        "description": "Maximum number of lines to read for local text files. Uses server default if not set.",
        // Making it truly optional by not setting a default here, let the handler use config default.
    }));
    ToolInputSchema::new(vec!["path".to_string()], Some(properties))
}

pub fn read_multiple_files_schema() -> ToolInputSchema {
    let mut properties = HashMap::new();
    properties.insert("paths".to_string(), create_array_prop("string", &format!("Array of file paths to read. {}", PATH_GUIDANCE)));
    ToolInputSchema::new(vec!["paths".to_string()], Some(properties))
}


pub fn write_file_schema() -> ToolInputSchema {
    let mut properties = HashMap::new();
    properties.insert("path".to_string(), create_prop("string", &format!("Path to the file. {}", PATH_GUIDANCE)));
    properties.insert("content".to_string(), create_prop("string", "Content to write. Adhere to server's fileWriteLineLimit."));
    properties.insert("mode".to_string(), create_enum_prop(vec!["rewrite", "append"], "rewrite", "Write mode: 'rewrite' or 'append'."));
    ToolInputSchema::new(vec!["path".to_string(), "content".to_string()], Some(properties))
}

pub fn create_directory_schema() -> ToolInputSchema {
    let mut properties = HashMap::new();
    properties.insert("path".to_string(), create_prop("string", &format!("Path of the directory to create. Can be nested. {}", PATH_GUIDANCE)));
    ToolInputSchema::new(vec!["path".to_string()], Some(properties))
}

pub fn list_directory_schema() -> ToolInputSchema {
    let mut properties = HashMap::new();
    properties.insert("path".to_string(), create_prop("string", &format!("Path of the directory to list. {}", PATH_GUIDANCE)));
    ToolInputSchema::new(vec!["path".to_string()], Some(properties))
}

pub fn move_file_schema() -> ToolInputSchema {
    let mut properties = HashMap::new();
    properties.insert("source".to_string(), create_prop("string", &format!("Source path (file or directory). {}", PATH_GUIDANCE)));
    properties.insert("destination".to_string(), create_prop("string", &format!("Destination path. {}", PATH_GUIDANCE)));
    ToolInputSchema::new(vec!["source".to_string(), "destination".to_string()], Some(properties))
}

pub fn search_files_schema() -> ToolInputSchema {
    let mut properties = HashMap::new();
    properties.insert("path".to_string(), create_prop("string", &format!("Root path for search. {}", PATH_GUIDANCE)));
    properties.insert("pattern".to_string(), create_prop("string", "Case-insensitive substring to search for in file/directory names."));
    properties.insert("timeoutMs".to_string(), json!({
        "type": "integer",
        "description": "Timeout in milliseconds. Default 30000 (30s).",
        // "optional": true // Optionality is handled by Option<T> in struct
    }));
    ToolInputSchema::new(vec!["path".to_string(), "pattern".to_string()], Some(properties))
}

pub fn get_file_info_schema() -> ToolInputSchema {
    let mut properties = HashMap::new();
    properties.insert("path".to_string(), create_prop("string", &format!("Path to the file or directory. {}", PATH_GUIDANCE)));
    ToolInputSchema::new(vec!["path".to_string()], Some(properties))
}

pub fn search_code_schema() -> ToolInputSchema {
    let mut properties = HashMap::new();
    properties.insert("pattern".to_string(), create_prop("string", "Search pattern (regex or literal string)."));
    properties.insert("path".to_string(), create_prop_with_default_str("string", &format!("Directory to search within. Relative to FILES_ROOT or absolute if allowed. Default is FILES_ROOT. {}", PATH_GUIDANCE), "."));
    properties.insert("fixed_strings".to_string(), create_prop_with_default_bool("boolean", "Treat pattern as literal string (no regex). Default false.", false));
    properties.insert("ignore_case".to_string(), create_prop_with_default_bool("boolean", "Perform case-insensitive search. Ripgrep's default is smart-case. This flag forces ignore-case. Default false.", false));
    properties.insert("case_sensitive".to_string(), create_prop_with_default_bool("boolean", "Perform case-sensitive search. Overrides ignore_case if both true. Default false.", false));
    properties.insert("line_numbers".to_string(), create_prop_with_default_bool("boolean", "Include line numbers in results. Default true.", true));
    properties.insert("context_lines".to_string(), json!({ "type": "integer", "description": "Number of context lines before and after matches. Default 0.", "default": 0_usize }));
    properties.insert("file_pattern".to_string(), json!({
        "type": "string",
        "description": "Glob pattern to filter files (e.g., \"*.rs\", \"!**/target/*\"). See ripgrep --glob.",
        // "optional": true
    }));
    properties.insert("max_depth".to_string(), json!({
        "type": "integer",
        "description": "Maximum search depth relative to the search path.",
        // "optional": true
    }));
    properties.insert("max_results".to_string(), create_prop_with_default_int("integer", "Maximum number of matches to return. Default 1000.", 1000));
    properties.insert("include_hidden".to_string(), create_prop_with_default_bool("boolean", "Include hidden files and directories in search. Default false.", false));
    properties.insert("timeoutMs".to_string(), json!({
        "type": "integer",
        "description": "Timeout in milliseconds. Default 30000 (30s).",
        // "optional": true
    }));
    ToolInputSchema::new(vec!["pattern".to_string()], Some(properties))
}

pub fn edit_block_schema() -> ToolInputSchema {
    let mut properties = HashMap::new();
    properties.insert("file_path".to_string(), create_prop("string", &format!("Path to the file. {}", PATH_GUIDANCE)));
    properties.insert("old_string".to_string(), create_prop("string", "The exact string to be replaced. Include enough context for uniqueness."));
    properties.insert("new_string".to_string(), create_prop("string", "The string to replace with."));
    properties.insert("expected_replacements".to_string(), create_prop_with_default_int("integer", "Number of occurrences expected to be replaced. If 0, attempts to replace all. Default 1.", 1));
    ToolInputSchema::new(
        vec!["file_path".to_string(), "old_string".to_string(), "new_string".to_string()],
        Some(properties),
    )
}

pub fn execute_command_schema() -> ToolInputSchema {
    let mut properties = HashMap::new();
    properties.insert("command".to_string(), create_prop("string", "The command to execute."));
    properties.insert("timeout_ms".to_string(), json!({
        "type": "integer",
        "description": "Timeout in milliseconds for initial output. Command continues in background if exceeded. Default 1000ms.",
        "default": 1000_u64, // Ensure type matches Option<u64>
        // "optional": true
    }));
    properties.insert("shell".to_string(), json!({
        "type": "string",
        "description": "Specific shell to use (e.g., 'bash', 'powershell'). Uses server's default shell if not set.",
        // "optional": true
    }));
    ToolInputSchema::new(vec!["command".to_string()], Some(properties))
}

pub fn read_output_schema() -> ToolInputSchema {
    let mut properties = HashMap::new();
    properties.insert("session_id".to_string(), create_prop("string", "ID of the command session obtained from execute_command."));
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

// Schema for set_config_value (used in handler.rs for deserialization)
#[derive(Debug, serde::Deserialize)]
pub struct SetConfigValueParams {
    pub key: String,
    pub value: Value,
}