use serde::Serialize;
use thiserror::Error;

#[derive(Error, Debug, Serialize)]
pub enum AppError {
    #[error("I/O error: {0}")]
    StdIoError(String), // Store as String for Serialize

    #[error("Tokio I/O error: {0}")]
    TokioIoError(String), // Store as String for Serialize

    #[error("Ripgrep error: {0}")]
    RipgrepError(String),

    #[error("Path traversal attempt: {0}")]
    PathTraversal(String),

    #[error("Path not allowed: {0}")]
    PathNotAllowed(String),

    #[error("Invalid path: {0}")]
    InvalidPath(String),

    #[error("Configuration error: {0}")]
    ConfigError(String), // Store as String for Serialize

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
    SerdeJsonError(String), // Store as String for Serialize

    #[cfg(feature = "sse")] // This might be removed if SSE is not part of Tauri app
    #[error("Hyper error: {0}")]
    HyperError(String), // Store as String for Serialize

    #[error("Reqwest HTTP error: {0}")]
    ReqwestError(String), // Store as String for Serialize

    #[error("Operation timed out: {0}")]
    TimeoutError(String),

    #[error("Invalid input argument: {0}")]
    InvalidInputArgument(String),

    #[error("Tauri API error: {0}")]
    TauriApiError(String),

    #[error("Plugin error ({plugin}): {message}")]
    PluginError { plugin: String, message: String },

    #[error("Unknown error: {0}")]
    Unknown(String),
}

// Implement From for various error types to AppError
// This helps in converting errors from dependencies into AppError easily using `?`

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        AppError::StdIoError(err.to_string())
    }
}

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
        AppError::ConfigError(format!("{:?}", err)) // Or a more specific variant
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

// Allows `Result<T, AppError>` to be used in Tauri commands
impl From<AppError> for String {
    fn from(error: AppError) -> Self {
        error.to_string()
    }
}