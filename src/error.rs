use thiserror::Error;

#[derive(Error, Debug)]
pub enum AppError {
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),

    // Removed #[from] to avoid conflict, handle explicitly or let it convert to std::io::Error
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

    #[error("Hyper error: {0}")]
    HyperError(#[from] hyper::Error),

    #[error("Reqwest HTTP error: {0}")]
    ReqwestError(#[from] reqwest::Error),

    #[error("Operation timed out: {0}")]
    TimeoutError(String),

    #[error("Invalid argument: {0}")]
    InvalidInputArgument(String),
}

impl From<AppError> for rust_mcp_schema::schema_utils::CallToolError {
    fn from(err: AppError) -> Self {
        tracing::error!("AppError occurred: {:?}", err);
        // Use a more specific error kind if possible, or Other for general.
        // The important part is that err.to_string() becomes the message.
        rust_mcp_schema::schema_utils::CallToolError::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            err.to_string(),
        ))
    }
}

// If you need to convert tokio::io::Error to AppError frequently:
impl From<tokio::io::Error> for AppError {
    fn from(err: tokio::io::Error) -> Self {
        AppError::TokioIoError(err)
    }
}