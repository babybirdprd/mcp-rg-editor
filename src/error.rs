use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Tokio I/O error: {0}")]
    TokioIoError(#[from] tokio::io::Error),

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
    SessionNotFound(String), // Changed from u32 to String for UUIDs

    #[error("Edit error: {0}")]
    EditError(String),

    #[error("Serde JSON error: {0}")]
    SerdeJsonError(#[from] serde_json::Error),

    #[error("Hyper error: {0}")]
    HyperError(#[from] hyper::Error), // If SSE is used

    #[error("Reqwest HTTP error: {0}")]
    ReqwestError(#[from] reqwest::Error), // For URL reading

    #[error("Operation timed out: {0}")]
    TimeoutError(String),

    #[error("Invalid argument: {0}")]
    InvalidInputArgument(String),
}

// Helper to convert AppError to CallToolError for MCP responses
impl From<AppError> for rust_mcp_schema::schema_utils::CallToolError {
    fn from(err: AppError) -> Self {
        // Log the full error internally for debugging before sending a potentially simplified one to Claude
        tracing::error!("AppError occurred: {:?}", err);
        rust_mcp_schema::schema_utils::CallToolError::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            err.to_string(), // This will be the message Claude sees
        ))
    }
}