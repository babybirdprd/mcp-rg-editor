// FILE: src-tauri/src/mcp/tool_impl/mod.rs
// IMPORTANT NOTE: Create this new file.
// This module will contain the actual implementations of the tool logic
// that the MCP handler will call. These implementations will use Tauri plugins.

pub mod filesystem;
pub mod ripgrep;
pub mod terminal;
pub mod process; 
pub mod edit;