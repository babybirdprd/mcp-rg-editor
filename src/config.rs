use anyhow::{Context, Result};
use std::path::PathBuf;
use std::str::FromStr;
use regex::Regex;

#[derive(Debug, Clone)]
pub struct Config {
    pub files_root: PathBuf,
    pub allowed_directories: Vec<PathBuf>,
    pub blocked_commands: Vec<Regex>,
    pub default_shell: Option<String>,
    pub log_level: String,
    pub transport_mode: TransportMode,
    pub sse_host: String,
    pub sse_port: u16,
    pub file_read_line_limit: usize,
    pub file_write_line_limit: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TransportMode {
    Stdio,
    Sse,
}

impl FromStr for TransportMode {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "stdio" => Ok(TransportMode::Stdio),
            "sse" => Ok(TransportMode::Sse),
            _ => Err(anyhow::anyhow!("Invalid transport mode: {}", s)),
        }
    }
}

impl Config {
    pub fn load() -> Result<Self> {
        dotenvy::dotenv().ok(); // Load .env file if present

        let files_root_str = std::env::var("FILES_ROOT").context("FILES_ROOT must be set")?;
        let files_root = PathBuf::from(files_root_str)
            .canonicalize()
            .context("Failed to canonicalize FILES_ROOT")?;
        if !files_root.is_dir() {
            anyhow::bail!("FILES_ROOT is not a valid directory: {:?}", files_root);
        }

        let allowed_directories_str = std::env::var("ALLOWED_DIRECTORIES").unwrap_or_default();
        let mut allowed_directories: Vec<PathBuf> = if allowed_directories_str.is_empty() {
            vec![files_root.clone()]
        } else {
            allowed_directories_str
                .split(',')
                .map(|s| PathBuf::from(s.trim()))
                .filter_map(|p| p.canonicalize().ok()) // Keep only valid, existing, absolute paths
                .collect()
        };
        // Ensure files_root is always allowed if other directories are specified
        if !allowed_directories.iter().any(|ad| ad == &files_root) {
             allowed_directories.push(files_root.clone());
        }


        let blocked_commands_str = std::env::var("BLOCKED_COMMANDS").unwrap_or_default();
        let blocked_commands = blocked_commands_str
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| Regex::new(&format!(r"^{}$", regex::escape(s))).context(format!("Invalid regex for blocked command: {}", s)))
            .collect::<Result<Vec<Regex>>>()?;

        let default_shell = std::env::var("DEFAULT_SHELL").ok().filter(|s| !s.is_empty());

        let log_level = std::env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string());

        let transport_mode_str = std::env::var("MCP_TRANSPORT").unwrap_or_else(|_| "stdio".to_string());
        let transport_mode = TransportMode::from_str(&transport_mode_str)?;

        let sse_host = std::env::var("MCP_SSE_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
        let sse_port = std::env::var("MCP_SSE_PORT")
            .unwrap_or_else(|_| "3000".to_string())
            .parse::<u16>()
            .context("Invalid MCP_SSE_PORT")?;

        let file_read_line_limit = std::env::var("FILE_READ_LINE_LIMIT")
            .unwrap_or_else(|_| "1000".to_string())
            .parse::<usize>()
            .context("Invalid FILE_READ_LINE_LIMIT")?;

        let file_write_line_limit = std::env::var("FILE_WRITE_LINE_LIMIT")
            .unwrap_or_else(|_| "50".to_string())
            .parse::<usize>()
            .context("Invalid FILE_WRITE_LINE_LIMIT")?;


        Ok(Config {
            files_root,
            allowed_directories,
            blocked_commands,
            default_shell,
            log_level,
            transport_mode,
            sse_host,
            sse_port,
            file_read_line_limit,
            file_write_line_limit,
        })
    }
}