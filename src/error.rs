// FILE: src/error.rs
use thiserror::Error;
use rust_mcp_schema::RpcErrorCode; // Added this import

#[derive(Error, Debug)]
pub enum AppError {
    #[error("I/O error: {0}")]
    StdIoError(std::io::Error), // Removed #[from]

    #[error("Tokio I/O error: {0}")]
    TokioIoError(tokio::io::Error), // Removed #[from]

    #[error("Ripgrep error: {0}")]
    RipgrepError(String),

    #[error("Path traversal attempt: {0}")]
    PathTraversal(String),

    #[error("Path not allowed: {0}")]
    PathNotAllowed(String),

    #[error("Invalid path: {0}")]
    InvalidPath(String),

    #[error("Configuration error: {0}")]
    ConfigError(#[from] anyhow::Error),

    #[error("MCP error: {0}")]
    MCPError(String), // General MCP related errors

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
    SerdeJsonError(#[from] serde_json::Error),

    #[cfg(feature = "sse")]
    #[error("Hyper error: {0}")]
    HyperError(#[from] hyper::Error), // Conditional compilation

    #[error("Reqwest HTTP error: {0}")]
    ReqwestError(#[from] reqwest::Error),

    #[error("Operation timed out: {0}")]
    TimeoutError(String),

    #[error("Invalid input argument: {0}")]
    InvalidInputArgument(String),
}

impl From<AppError> for rust_mcp_schema::schema_utils::CallToolError {
    fn from(err: AppError) -> Self {
        tracing::error!("AppError converted to CallToolError: {:?}", err);
        // Use a standard RpcError for CallToolError
        let rpc_error = match err {
            AppError::InvalidInputArgument(_) | AppError::PathNotAllowed(_) | AppError::PathTraversal(_) | AppError::InvalidPath(_) =>
                rust_mcp_schema::RpcError::new(
                    RpcErrorCode::InvalidParams, // Corrected: RpcErrorCode from rust_mcp_schema
                    err.to_string(),
                    None,
                ),
            AppError::CommandBlocked(_) =>
                rust_mcp_schema::RpcError::new(
                    RpcErrorCode::ServerError(-32001), // Corrected: RpcErrorCode from rust_mcp_schema
                    err.to_string(),
                    None,
                ),
            _ => rust_mcp_schema::RpcError::new(
                RpcErrorCode::InternalError, // Corrected: RpcErrorCode from rust_mcp_schema
                err.to_string(),
                None,
            ),
        };
        rust_mcp_schema::schema_utils::CallToolError::new(rpc_error)
    }
}

// Explicit From impl for std::io::Error
impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        AppError::StdIoError(err)
    }
}

// Explicit conversion from tokio::io::Error to AppError
impl From<tokio::io::Error> for AppError {
    fn from(err: tokio::io::Error) -> Self {
        AppError::TokioIoError(err)
    }
}