use crate::config::Config;
use anyhow::Result;
use chrono::Utc;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::{Arc, RwLock as StdRwLock};
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex as TokioMutex;
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
    initialized: TokioMutex<bool>,
     max_size_bytes: u64, // Added for rotation
}

impl FuzzySearchLogger {
    pub fn new(config_state: Arc<StdRwLock<Config>>) -> Self {
        let config_guard = config_state.read().unwrap();
        let log_file_path = config_guard.fuzzy_search_log_file.clone();
        // Using audit log's max size for fuzzy log as well, or define a new env var
        let max_size_bytes = config_guard.audit_log_max_size_bytes;
        drop(config_guard);

        if let Some(parent_dir) = log_file_path.parent() {
            if !parent_dir.exists() {
                if let Err(e) = std::fs::create_dir_all(parent_dir) {
                    error!(path = %parent_dir.display(), error = %e, "Failed to create fuzzy search log directory");
                }
            }
        }
        Self {
            log_file_path,
            initialized: TokioMutex::new(false),
            max_size_bytes,
        }
    }

    async fn rotate_log_if_needed(&self) -> Result<()> {
        if !self.log_file_path.exists() {
            return Ok(());
        }
        let metadata = fs::metadata(&self.log_file_path).await?;
        if metadata.len() >= self.max_size_bytes {
            let timestamp = Utc::now().format("%Y-%m-%d_%H-%M-%S").to_string();
            let file_stem = self.log_file_path.file_stem().unwrap_or_default().to_string_lossy();
            let extension = self.log_file_path.extension().unwrap_or_default().to_string_lossy();
            let backup_file_name = format!("{}_{}.{}", file_stem, timestamp, extension);
            let backup_path = self.log_file_path.with_file_name(backup_file_name);
            fs::rename(&self.log_file_path, backup_path).await?;
            // After renaming, the original file is gone, so we need to re-initialize headers.
            let mut initialized_guard = self.initialized.lock().await;
            *initialized_guard = false;
        }
        Ok(())
    }


    async fn ensure_log_file_initialized(&self) -> Result<()> {
        let mut initialized_guard = self.initialized.lock().await;
        if *initialized_guard {
            return Ok(());
        }

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
            .join("\t"); // Use tab as delimiter for TSV
            let mut file = OpenOptions::new()
                .create(true)
                .write(true)
                .open(&self.log_file_path)
                .await?;
            file.write_all(format!("{}\n", headers).as_bytes()).await?;
        }
        *initialized_guard = true;
        Ok(())
    }

    pub async fn log(&self, entry: &FuzzySearchLogEntry) {
        if let Err(e) = self.try_log(entry).await {
            error!(error = %e, "Failed to write fuzzy search log");
        }
    }

    async fn try_log(&self, entry: &FuzzySearchLogEntry) -> Result<()> {
        self.rotate_log_if_needed().await?; // Check for rotation before ensuring initialization
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
            .open(&self.log_file_path)
            .await?;
        file.write_all(log_line.as_bytes()).await?;
        Ok(())
    }
}