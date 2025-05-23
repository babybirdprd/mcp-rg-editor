use anyhow::{Context, Result};
use regex::Regex;
use shellexpand;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use tauri::Manager;
use tracing::{info, warn}; // Added info

// New struct UserAppSettings
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UserAppSettings {
    pub files_root: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Config {
    pub files_root: PathBuf,
    pub allowed_directories: Vec<PathBuf>,
    pub blocked_commands: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_shell: Option<String>,
    pub log_level: String,
    pub mcp_transport_mode: TransportMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_sse_host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mcp_sse_port: Option<u16>,
    pub file_read_line_limit: usize,
    pub file_write_line_limit: usize,
    pub audit_log_file: PathBuf,
    pub audit_log_max_size_bytes: u64,
    pub fuzzy_search_log_file: PathBuf,
    pub mcp_log_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)] // Added Eq
pub enum TransportMode {
    Stdio,
    Sse,
    Disabled,
}

impl FromStr for TransportMode {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "stdio" => Ok(TransportMode::Stdio),
            "sse" => Ok(TransportMode::Sse),
            "disabled" => Ok(TransportMode::Disabled),
            _ => Err(anyhow::anyhow!("Invalid MCP transport mode: {}. Valid options are 'stdio', 'sse', 'disabled'.", s)),
        }
    }
}

pub fn expand_tilde(path_str: &str) -> Result<PathBuf, anyhow::Error> {
    Ok(PathBuf::from(shellexpand::tilde(path_str).as_ref()))
}

impl Config {
    pub fn load(app_handle: &tauri::AppHandle) -> Result<Self> {
        dotenvy::dotenv().ok();

        let app_config_dir = app_handle.path().app_config_dir()?;
        let settings_path = app_config_dir.join("settings.json");
        
        let mut files_root_path_opt: Option<PathBuf> = None;
        let mut source_of_files_root: String = "default".to_string(); // Tracks origin: "settings", "env", "default"
        
        let mut settings_file_existed = false;
        let mut settings_file_was_readable_and_parsable = false;
        let mut settings_file_had_valid_files_root_entry = false;

        if settings_path.exists() && settings_path.is_file() {
            settings_file_existed = true;
            match fs::read_to_string(&settings_path) {
                Ok(settings_content) => {
                    match serde_json::from_str::<UserAppSettings>(&settings_content) {
                        Ok(user_app_settings) => {
                            settings_file_was_readable_and_parsable = true;
                            if let Some(path_str) = user_app_settings.files_root {
                                if !path_str.trim().is_empty() {
                                    match expand_tilde(&path_str) {
                                        Ok(expanded_path) => {
                                            files_root_path_opt = Some(expanded_path);
                                            source_of_files_root = "settings".to_string();
                                            settings_file_had_valid_files_root_entry = true;
                                            info!("Loaded 'files_root' from settings.json: {}", path_str);
                                        }
                                        Err(e) => {
                                            warn!("Failed to expand tilde for 'files_root' from settings.json ('{}'): {}. Will check environment variable.", path_str, e);
                                        }
                                    }
                                } else {
                                    info!("'files_root' in settings.json is empty. Will check environment variable.");
                                }
                            } else {
                                info!("No 'files_root' key in settings.json. Will check environment variable.");
                            }
                        }
                        Err(e) => {
                            warn!("Failed to deserialize settings.json: {}. Will check environment variable.", e);
                        }
                    }
                }
                Err(e) => {
                    warn!("Failed to read settings.json: {}. Will check environment variable.", e);
                }
            }
        } else {
            info!("settings.json not found. Will check environment variable.");
        }

        if files_root_path_opt.is_none() {
            if let Some(env_var_value) = std::env::var("FILES_ROOT").ok().filter(|s| !s.is_empty()) {
                match expand_tilde(&env_var_value) {
                    Ok(expanded_path) => {
                        files_root_path_opt = Some(expanded_path);
                        source_of_files_root = "env".to_string();
                        info!("Using FILES_ROOT from environment variable: {}", env_var_value);
                    }
                    Err(e) => {
                        warn!("Failed to expand tilde for FILES_ROOT environment variable ('{}'): {}. Will use default.", env_var_value, e);
                        // files_root_path_opt remains None, source_of_files_root will become "default"
                    }
                }
            }
        }

        let initial_files_root: PathBuf;
        if let Some(path) = files_root_path_opt {
            initial_files_root = path;
        } else {
            // Source is already "default" if files_root_path_opt is None here
            let home_dir = dirs_next::home_dir().context("Failed to get user home directory for default files_root")?;
            initial_files_root = home_dir.join("rg-editor/");
            source_of_files_root = "default".to_string(); // Explicitly set for clarity
            info!("Using default files_root: {}", initial_files_root.display());
        }
        
        let files_root = initial_files_root.canonicalize().or_else(|e| {
            warn!(path = %initial_files_root.display(), error = %e, "Initial files_root failed to canonicalize, attempting to create it.");
            fs::create_dir_all(&initial_files_root).context(format!("Failed to create initial files_root: {}", initial_files_root.display()))?;
            initial_files_root.canonicalize().context(format!("Failed to canonicalize initial files_root after creation: {}", initial_files_root.display()))
        })?;

        if !files_root.is_dir() {
            anyhow::bail!("Effective files_root is not a valid directory: {:?}", files_root);
        }

        // Determine if we need to save to settings.json
        let should_save_to_settings = !settings_file_existed || 
                                      (settings_file_existed && !settings_file_was_readable_and_parsable) ||
                                      (settings_file_existed && settings_file_was_readable_and_parsable && !settings_file_had_valid_files_root_entry) ||
                                      source_of_files_root == "env" || 
                                      source_of_files_root == "default";

        if should_save_to_settings {
            // Ensure parent directory for settings.json exists
            if let Some(parent_dir) = settings_path.parent() {
                if !parent_dir.exists() {
                    fs::create_dir_all(parent_dir).context(format!("Failed to create parent directory for settings.json: {}", parent_dir.display()))?;
                }
            }
            
            let app_settings_to_save = UserAppSettings {
                files_root: Some(files_root.to_string_lossy().into_owned()),
            };
            match serde_json::to_string_pretty(&app_settings_to_save) {
                Ok(json_string) => {
                    match fs::write(&settings_path, json_string) {
                        Ok(_) => info!("Saved files_root configuration to settings.json: {}", settings_path.display()),
                        Err(e) => warn!("Failed to write settings.json to {}: {}", settings_path.display(), e),
                    }
                }
                Err(e) => warn!("Failed to serialize UserAppSettings to JSON for saving: {}", e),
            }
        }

        let allowed_directories_str = std::env::var("ALLOWED_DIRECTORIES").unwrap_or_default();
        let mut allowed_directories: Vec<PathBuf> = if allowed_directories_str.is_empty() {
            vec![files_root.clone()]
        } else if allowed_directories_str == "/" || (cfg!(windows) && Regex::new(r"^[a-zA-Z]:[\\/]?$").unwrap().is_match(&allowed_directories_str)) {
            warn!("ALLOWED_DIRECTORIES is set to full filesystem access ('{}'). This is highly permissive.", allowed_directories_str);
            vec![PathBuf::from(allowed_directories_str.trim_end_matches(|c| c == '/' || c == '\\'))]
        } else {
            allowed_directories_str
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| expand_tilde(s).context(format!("Failed to expand tilde for allowed_directory: {}", s)))
                .collect::<Result<Vec<PathBuf>>>()?
                .into_iter()
                .map(|p| p.canonicalize().unwrap_or_else(|_| p.clone()))
                .collect()
        };
        
        let is_files_root_broad = files_root == Path::new("/") || 
                                (cfg!(windows) && files_root.parent().is_none() && files_root.is_absolute());

        if !is_files_root_broad {
            if !allowed_directories.iter().any(|ad| ad == &files_root) {
                allowed_directories.push(files_root.clone());
            }
        }
        allowed_directories.sort();
        allowed_directories.dedup();

        let blocked_commands_str = std::env::var("BLOCKED_COMMANDS")
            .unwrap_or_else(|_| "sudo,su,rm,mkfs,fdisk,dd,reboot,shutdown,poweroff,halt,format,mount,umount,passwd,adduser,useradd,usermod,groupadd".to_string());
        let blocked_commands = blocked_commands_str
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect::<Vec<String>>();

        let default_shell = std::env::var("DEFAULT_SHELL").ok().filter(|s| !s.is_empty());
        let log_level = std::env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string());

        let mcp_transport_mode_str = std::env::var("MCP_TRANSPORT").unwrap_or_else(|_| {
            if cfg!(feature = "mcp-sse-server") { "sse".to_string() }
            else if cfg!(feature = "mcp-stdio-server") { "stdio".to_string() }
            else { "disabled".to_string() }
        });
        let mcp_transport_mode = TransportMode::from_str(&mcp_transport_mode_str)?;
        let mcp_sse_host = std::env::var("MCP_SSE_HOST").ok();
        let mcp_sse_port = std::env::var("MCP_SSE_PORT")
            .ok()
            .and_then(|s| s.parse::<u16>().ok());

        let file_read_line_limit = std::env::var("FILE_READ_LINE_LIMIT")
            .unwrap_or_else(|_| "1000".to_string())
            .parse::<usize>()
            .context("Invalid FILE_READ_LINE_LIMIT")?;
        let file_write_line_limit = std::env::var("FILE_WRITE_LINE_LIMIT")
            .unwrap_or_else(|_| "50".to_string())
            .parse::<usize>()
            .context("Invalid FILE_WRITE_LINE_LIMIT")?;
        
        let app_log_dir_base = app_handle.path().app_log_dir()
            .context("Failed to get app log directory from Tauri")?;

        let mcp_log_dir_env_var = std::env::var("MCP_LOG_DIR").ok();
        let mcp_log_dir_path = match mcp_log_dir_env_var {
            Some(dir_str) if !dir_str.is_empty() => expand_tilde(&dir_str)?,
            _ => app_log_dir_base.join("mcp-rg-editor-logs"),
        };

        if !mcp_log_dir_path.exists() {
            std::fs::create_dir_all(&mcp_log_dir_path).context(format!("Failed to create MCP_LOG_DIR: {}", mcp_log_dir_path.display()))?;
        }
        let mcp_log_dir = mcp_log_dir_path.canonicalize().context(format!("Failed to canonicalize MCP_LOG_DIR: {}", mcp_log_dir_path.display()))?;

        let audit_log_file = mcp_log_dir.join("audit_tool_calls.log");
        let audit_log_max_size_bytes = std::env::var("AUDIT_LOG_MAX_SIZE_MB")
            .unwrap_or_else(|_| "10".to_string())
            .parse::<u64>()
            .map(|mb| mb * 1024 * 1024) 
            .unwrap_or(10 * 1024 * 1024); 
        let fuzzy_search_log_file = mcp_log_dir.join("fuzzy_search_attempts.log");

        Ok(Config {
            files_root,
            allowed_directories,
            blocked_commands,
            default_shell,
            log_level,
            mcp_transport_mode,
            mcp_sse_host,
            mcp_sse_port,
            file_read_line_limit,
            file_write_line_limit,
            audit_log_file,
            audit_log_max_size_bytes,
            fuzzy_search_log_file,
            mcp_log_dir,
        })
    }

    pub fn get_blocked_command_regexes(&self) -> Result<Vec<Regex>> {
        self.blocked_commands
            .iter()
            .map(|s| Regex::new(&format!(r"^(?:[a-zA-Z_][a-zA-Z0-9_]*=[^ ]* )*{}(?:\s.*|$)", regex::escape(s)))
                .context(format!("Invalid regex for blocked command: {}", s)))
            .collect()
    }
}

pub fn init_config_state(app_handle: &tauri::AppHandle) -> Result<std::sync::Arc<std::sync::RwLock<Config>>, anyhow::Error> {
    Config::load(app_handle).map(|config| std::sync::Arc::new(std::sync::RwLock::new(config)))
}