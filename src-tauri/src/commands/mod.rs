
pub mod config_commands;
pub mod filesystem_commands;
pub mod ripgrep_commands;
pub mod terminal_commands;
pub mod process_commands;
pub mod edit_commands;

// A simple greet command for initial testing
#[tauri::command]
pub fn greet() -> String {
  let now = std::time::SystemTime::now();
  match now.duration_since(std::time::UNIX_EPOCH) {
    Ok(n) => format!("Hello from Rust (Tauri)! Current epoch ms: {}", n.as_millis()),
    Err(_) => "Hello from Rust (Tauri)! Could not get epoch time.".to_string(),
  }
}