// FILE: src/utils/fuzzy_search_logger.rs
use crate::config::Config;
use anyhow::Result;
use chrono::Utc;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::{Arc, RwLock as StdRwLock}; 
use tokio::fs::OpenOptions; // Corrected import
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex as TokioMutex; // For internal mutability
use tracing::error;

#[derive(Debug, Serialize)]
pub struct FuzzySearchLogEntry {
    pub timestamp: chrono::DateTime<Utc>,
    pub search_text: String,
    pub found_text: String,
    pub similarity: f64,
    pub execution_time_ms: f64,
    pub exact_match_count: usize,
    pub expected_replacements: usize,
    pub fuzzy_threshold: f64,
    pub below_threshold: bool,
    pub diff: String,
    pub search_length: usize,
    pub found_length: usize,
    pub file_extension: String,
    pub character_codes: String,
    pub unique_character_count: usize,
    pub diff_length: usize,
}

#[derive(Debug)] 
pub struct FuzzySearchLogger {
    log_file_path: PathBuf,
    initialized: TokioMutex<bool>, // Changed to TokioMutex<bool>
}

impl FuzzySearchLogger {
    pub fn new(config: Arc<StdRwLock<Config>>) -> Self { 
        let config_guard = config.read().unwrap(); 
        let log_file_path = config_guard.fuzzy_search_log_file.clone();
        drop(config_guard); 

        if let Some(parent_dir) = log_file_path.parent() {
            if !parent_dir.exists() {
                // Use std::fs for synchronous operations in constructor
                if let Err(e) = std::fs::create_dir_all(parent_dir) {
                     error!(path = %parent_dir.display(), error = %e, "Failed to create fuzzy search log directory");
                }
            }
        }
        Self {
            log_file_path,
            initialized: TokioMutex::new(false), // Initialize Mutex
        }
    }

    async fn ensure_log_file_initialized(&self) -> Result<()> { // Takes &self
        let mut initialized_guard = self.initialized.lock().await; // Lock the mutex
        if *initialized_guard {
            return Ok(());
        }

        // Check existence asynchronously
        let exists = match tokio::fs::metadata(&self.log_file_path).await {
            Ok(_) => true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => return Err(e.into()),
        };

        if !exists {
            let headers = [
                "timestamp", "searchText", "foundText", "similarity",
                "executionTime_ms", "exactMatchCount", "expectedReplacements",
                "fuzzyThreshold", "belowThreshold", "diff", "searchLength",
                "foundLength", "fileExtension", "characterCodes",
                "uniqueCharacterCount", "diffLength",
            ]
            .join("\t");
            let mut file = OpenOptions::new()
                .create(true)
                .write(true)
                .open(&self.log_file_path)
                .await?;
            file.write_all(format!("{}\n", headers).as_bytes()).await?;
        }
        *initialized_guard = true; // Mutate through guard
        Ok(())
    }

    pub async fn log(&self, entry: &FuzzySearchLogEntry) -> Result<()> { // Takes &self
        if let Err(e) = self.try_log(entry).await {
            error!(error = %e, "Failed to write fuzzy search log");
            return Err(e); 
        }
        Ok(())
    }

    async fn try_log(&self, entry: &FuzzySearchLogEntry) -> Result<()> { // Takes &self
        self.ensure_log_file_initialized().await?;

        let escape = |s: &str| s.replace('\t', "\\t").replace('\n', "\\n").replace('\r', "\\r");

        let log_line = format!(
            "{}\t{}\t{}\t{:.4}\t{:.2}\t{}\t{}\t{:.2}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            entry.timestamp.to_rfc3339(),
            escape(&entry.search_text),
            escape(&entry.found_text),
            entry.similarity,
            entry.execution_time_ms,
            entry.exact_match_count,
            entry.expected_replacements,
            entry.fuzzy_threshold,
            entry.below_threshold,
            escape(&entry.diff),
            entry.search_length,
            entry.found_length,
            escape(&entry.file_extension),
            escape(&entry.character_codes),
            entry.unique_character_count,
            entry.diff_length
        );

        let mut file = OpenOptions::new()
            .append(true)
            .open(&self.log_file_path) // This OpenOptions is tokio::fs::OpenOptions
            .await?;
        file.write_all(log_line.as_bytes()).await?;
        Ok(())
    }
}