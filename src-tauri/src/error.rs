// FILE: src-tauri/src/error.rs
use serde::Serialize;
use thiserror::Error;

#[derive(Error, Debug, Serialize)]
pub enum AppError {
    #[error("I/O error: {0}")]
    StdIoError(String), // Changed from std::io::Error to String for Serialize

    #[error("Tokio I/O error: {0}")]
    TokioIoError(String), // Changed from tokio::io::Error to String for Serialize

    #[error("Ripgrep error: {0}")]
    RipgrepError(String),

    #[error("Path traversal attempt: {0}")]
    PathTraversal(String),

    #[error("Path not allowed: {0}")]
    PathNotAllowed(String),

    #[error("Invalid path: {0}")]
    InvalidPath(String),

    #[error("Configuration error: {0}")]
    ConfigError(String), // Changed from anyhow::Error

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
    SerdeJsonError(String), // Changed from serde_json::Error

    #[error("Reqwest HTTP error: {0}")]
    ReqwestError(String), // Changed from reqwest::Error

    #[error("Operation timed out: {0}")]
    TimeoutError(String),

    #[error("Invalid input argument: {0}")]
    InvalidInputArgument(String),

    #[error("Tauri API error: {0}")]
    TauriApiError(String), // Changed from tauri::Error

    #[error("Plugin error ({plugin}): {message}")]
    PluginError { plugin: String, message: String },

    #[error("Unknown error: {0}")]
    Unknown(String),
}

// Implement From for common error types to AppError, converting them to String.
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
        // Using format! to capture the full context of anyhow::Error
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
        AppError::TauriApiError(format!("{:?}", err)) // tauri::Error might not be simple string
    }
}

// Allows `Result<T, AppError>` to be used in Tauri commands
// by converting AppError into a String that Tauri can serialize.
impl From<AppError> for String {
    fn from(error: AppError) -> Self {
        error.to_string()
    }
}