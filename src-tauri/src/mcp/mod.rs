// FILE: src-tauri/src/mcp/mod.rs
// IMPORTANT NOTE: Rewrite the entire file.
pub mod handler;
pub mod schemas;
pub mod tool_impl; // To store actual tool logic implementations

use tauri::AppHandle;
use std::sync::{Arc, RwLock};
use crate::config::Config;

// Struct to pass to the MCP Server thread
#[derive(Clone)]
pub struct McpServerLaunchParams {
    pub app_handle: AppHandle,
    pub config_state: Arc<RwLock<Config>>,
}