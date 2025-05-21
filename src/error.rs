// FILE: src/error.rs
use thiserror::Error;
use rust_mcp_schema::RpcErrorCode; // Corrected import path

#[derive(Error, Debug)]
pub enum AppError {
    #[error("I/O error: {0}")]
    StdIoError(std::io::Error), 

    #[error("Tokio I/O error: {0}")]
    TokioIoError(tokio::io::Error), 

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
    MCPError(String), 

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
    HyperError(#[from] hyper::Error), 

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
        let rpc_error = match err {
            AppError::InvalidInputArgument(_) | AppError::PathNotAllowed(_) | AppError::PathTraversal(_) | AppError::InvalidPath(_) =>
                rust_mcp_schema::RpcError::new(
                    RpcErrorCode::InvalidParams, 
                    err.to_string(),
                    None,
                ),
            AppError::CommandBlocked(_) =>
                rust_mcp_schema::RpcError::new(
                    RpcErrorCode::ServerError(-32001), 
                    err.to_string(),
                    None,
                ),
            _ => rust_mcp_schema::RpcError::new(
                RpcErrorCode::InternalError, 
                err.to_string(),
                None,
            ),
        };
        rust_mcp_schema::schema_utils::CallToolError::new(rpc_error)
    }
}

// Removed: impl From<std::io::Error> for AppError to resolve E0119
// std::io::Error can be converted to tokio::io::Error, which then uses the From<tokio::io::Error> impl.

impl From<tokio::io::Error> for AppError {
    fn from(err: tokio::io::Error) -> Self {
        AppError::TokioIoError(err)
    }
}