pub mod config_commands;
pub mod filesystem_commands;
pub mod ripgrep_commands;
pub mod terminal_commands;
pub mod process_commands; // New
pub mod edit_commands;    // New

// Re-export the greet command from the template or your own version
#[tauri::command]
pub fn greet() -> String {
  let now = std::time::SystemTime::now();
  let epoch_ms = now.duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
  format!("Hello world from Rust (Tauri)! Current epoch: {}", epoch_ms)
}