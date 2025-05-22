pub mod handler;
pub mod schemas;
pub mod tool_impl;

use tauri::AppHandle;
use std::sync::{Arc, RwLock};
use crate::config::Config;

// Struct to pass to the MCP Server thread, if needed (currently handler takes AppHandle and Config directly)
#[derive(Clone)]
pub struct McpServerLaunchParams {
    pub app_handle: AppHandle,
    pub config_state: Arc<RwLock<Config>>,
}