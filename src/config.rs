use anyhow::{Context, Result};
use regex::Regex;
use shellexpand;
use std::path::PathBuf;
use std::str::FromStr;
use tracing::warn;
use uuid::Uuid;

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
    pub audit_log_file: PathBuf,
    pub audit_log_max_size_bytes: u64,
    pub fuzzy_search_log_file: PathBuf,
    // Telemetry related fields are removed as per request
    // pub telemetry_enabled: bool,
    // pub client_id: String,
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

fn expand_tilde(path_str: &str) -> Result<PathBuf, anyhow::Error> {
    shellexpand::tilde(path_str)
        .map(PathBuf::from)
        .map_err(|e| anyhow::anyhow!("Failed to expand tilde for path '{}': {}", path_str, e))
}

impl Config {
    pub fn load() -> Result<Self> {
        dotenvy::dotenv().ok(); // Load .env file if present

        let files_root_str = std::env::var("FILES_ROOT")
            .context("FILES_ROOT environment variable must be set")?;
        let files_root = expand_tilde(&files_root_str)?
            .canonicalize()
            .context(format!("Failed to canonicalize FILES_ROOT: {}", files_root_str))?;
        if !files_root.is_dir() {
            anyhow::bail!("FILES_ROOT is not a valid directory: {:?}", files_root);
        }

        let allowed_directories_str = std::env::var("ALLOWED_DIRECTORIES").unwrap_or_default();
        let mut allowed_directories: Vec<PathBuf> = if allowed_directories_str.is_empty() {
            vec![files_root.clone()]
        } else if allowed_directories_str == "/" || (cfg!(windows) && regex::Regex::new(r"^[a-zA-Z]:\\?$").unwrap().is_match(&allowed_directories_str)) {
            // Special case: allow full filesystem access if "/" or "C:\" is specified
            // This is dangerous and should be used with caution.
            warn!("ALLOWED_DIRECTORIES is set to full filesystem access ('{}'). This is highly permissive.", allowed_directories_str);
            vec![PathBuf::from(allowed_directories_str)] // Store it as is, validation will handle it
        } else {
            allowed_directories_str
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| expand_tilde(s))
                .filter_map(Result::ok) // Keep only successfully expanded paths
                .filter_map(|p| p.canonicalize().ok()) // Keep only valid, existing, absolute paths
                .collect()
        };

        // Ensure files_root is always implicitly allowed if other directories are specified and files_root itself isn't "/"
        if files_root != PathBuf::from("/") && (cfg!(windows) && !regex::Regex::new(r"^[a-zA-Z]:\\?$").unwrap().is_match(files_root.to_str().unwrap_or_default())) {
            if !allowed_directories.iter().any(|ad| ad == &files_root) {
                allowed_directories.push(files_root.clone());
            }
        }
        // Remove duplicates
        allowed_directories.sort();
        allowed_directories.dedup();


        let blocked_commands_str = std::env::var("BLOCKED_COMMANDS")
            .unwrap_or_else(|_| "sudo,su,rm,mkfs,fdisk,dd,reboot,shutdown,poweroff,halt,format,mount,umount,passwd,adduser,useradd,usermod,groupadd".to_string());
        let blocked_commands = blocked_commands_str
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            // Match command if it's the first word, possibly preceded by env vars
            .map(|s| Regex::new(&format!(r"^(?:[a-zA-Z_][a-zA-Z0-9_]*=[^ ]* )*{}(?:\s.*|$)", regex::escape(s))).context(format!("Invalid regex for blocked command: {}", s)))
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

        // Log file locations (can be made configurable via env vars if needed)
        let log_dir_base = std::env::var("MCP_LOG_DIR")
            .ok()
            .and_then(|s| expand_tilde(&s).ok())
            .unwrap_or_else(|| files_root.join(".mcp-logs"));


        let audit_log_file = log_dir_base.join("tool_calls.log");
        let audit_log_max_size_bytes = std::env::var("AUDIT_LOG_MAX_SIZE_MB")
            .unwrap_or_else(|_| "10".to_string()) // Default 10 MB
            .parse::<u64>()
            .map(|mb| mb * 1024 * 1024) // Convert MB to Bytes
            .unwrap_or(10 * 1024 * 1024);


        let fuzzy_search_log_file = log_dir_base.join("fuzzy-search.log");

        // Telemetry related fields removed
        // let telemetry_enabled = std::env::var("TELEMETRY_ENABLED")
        //     .unwrap_or_else(|_| "true".to_string())
        //     .parse::<bool>()
        //     .unwrap_or(true);
        // let client_id = std::env::var("CLIENT_ID").unwrap_or_else(|_| Uuid::new_v4().to_string());


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
            audit_log_file,
            audit_log_max_size_bytes,
            fuzzy_search_log_file,
            // telemetry_enabled,
            // client_id,
        })
    }

    // Helper to get a mutable reference to specific fields if needed for set_config_value
    // This would require Config to be wrapped in Arc<RwLock<Config>> in AppManagers
    // For now, set_config_value will be more complex or limited to in-memory changes.
}