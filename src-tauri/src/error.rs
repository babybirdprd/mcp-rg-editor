use serde::Serialize;
use thiserror::Error;

#[derive(Error, Debug, Serialize)]
pub enum AppError {
    #[error("I/O error: {0}")]
    StdIoError(String),

    #[error("Tokio I/O error: {0}")]
    TokioIoError(String),

    #[error("Ripgrep error: {0}")]
    RipgrepError(String),

    #[error("Path traversal attempt: {0}")]
    PathTraversal(String),

    #[error("Path not allowed: {0}")]
    PathNotAllowed(String),

    #[error("Invalid path: {0}")]
    InvalidPath(String),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("Command execution error: {0}")]
    CommandExecutionError(String),

    #[error("Command blocked: {0}")]
    CommandBlocked(String),

    #[error("Process error: {0}")]
    ProcessError(String),

    #[error("Session not found for ID: {0}")]
    SessionNotFound(String),

    #[error("Edit error: {0}")]
    EditError(String),

    #[error("Serde JSON error: {0}")]
    SerdeJsonError(String),

    #[error("Reqwest HTTP error: {0}")]
    ReqwestError(String),

    #[error("Operation timed out: {0}")]
    TimeoutError(String),

    #[error("Invalid input argument: {0}")]
    InvalidInputArgument(String),

    #[error("Tauri API error: {0}")]
    TauriApiError(String),

    #[error("Tauri Plugin error ({plugin}): {message}")]
    PluginError { plugin: String, message: String },

    #[error("MCP SDK error: {0}")]
    McpSdkError(String),

    #[error("Unknown error: {0}")]
    Unknown(String),
}

// Removed: impl From<std::io::Error> for AppError to resolve conflict.
// Manually map std::io::Error where needed: .map_err(|e| AppError::StdIoError(e.to_string()))

impl From<tokio::io::Error> for AppError {
    fn from(err: tokio::io::Error) -> Self {
        AppError::TokioIoError(err.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(err: serde_json::Error) -> Self {
        AppError::SerdeJsonError(err.to_string())
    }
}

impl From<anyhow::Error> for AppError {
    fn from(err: anyhow::Error) -> Self {
        // Attempt to downcast to AppError first to avoid wrapping AppError in AppError
        if let Some(app_err) = err.downcast_ref::<AppError>() {
            // This might involve cloning or a more sophisticated way to handle it
            // For now, let's just re-serialize its string representation if it's already AppError
            // A better approach might be to ensure AppError is not wrapped by anyhow in the first place
            // or to have a more direct way to extract it.
            // Cloning the AppError if it's cloneable is better.
            // For simplicity, using its string representation for now if not Clone.
            // If AppError becomes Clone: return app_err.clone();
            return AppError::Unknown(format!("Wrapped AppError: {}", app_err));
        }
        AppError::ConfigError(format!("{:?}", err))
    }
}


impl From<reqwest::Error> for AppError {
    fn from(err: reqwest::Error) -> Self {
        AppError::ReqwestError(err.to_string())
    }
}

impl From<tauri::Error> for AppError {
    fn from(err: tauri::Error) -> Self {
        AppError::TauriApiError(format!("{:?}", err))
    }
}

impl From<rust_mcp_sdk::error::McpSdkError> for AppError {
    fn from(err: rust_mcp_sdk::error::McpSdkError) -> Self {
        AppError::McpSdkError(format!("{:?}", err))
    }
}

impl From<AppError> for String {
    fn from(error: AppError) -> Self {
        error.to_string()
    }
}