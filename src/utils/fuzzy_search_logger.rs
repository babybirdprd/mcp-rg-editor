use crate::config::Config;
use anyhow::Result;
use chrono::Utc;
use serde::Serialize; // For potential future use if we serialize to JSON log
use std::path::PathBuf;
use std::sync::Arc;
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;
use tracing::error;

#[derive(Debug, Serialize)] // Added Serialize for potential JSON logging
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
    pub character_codes: String, // e.g., "32:10,97:5" (code:count)
    pub unique_character_count: usize,
    pub diff_length: usize,
}

pub struct FuzzySearchLogger {
    log_file_path: PathBuf,
    initialized: bool,
}

impl FuzzySearchLogger {
    pub fn new(config: Arc<Config>) -> Self {
        // Ensure log directory exists
        if let Some(parent_dir) = config.fuzzy_search_log_file.parent() {
            if !parent_dir.exists() {
                if let Err(e) = std::fs::create_dir_all(parent_dir) {
                     error!(path = %parent_dir.display(), error = %e, "Failed to create fuzzy search log directory");
                }
            }
        }
        Self {
            log_file_path: config.fuzzy_search_log_file.clone(),
            initialized: false,
        }
    }

    async fn ensure_log_file_initialized(&mut self) -> Result<()> {
        if self.initialized {
            return Ok(());
        }

        if !self.log_file_path.exists() {
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
        self.initialized = true;
        Ok(())
    }

    pub async fn log(&mut self, entry: &FuzzySearchLogEntry) {
        if let Err(e) = self.try_log(entry).await {
            error!(error = %e, "Failed to write fuzzy search log");
        }
    }

    async fn try_log(&mut self, entry: &FuzzySearchLogEntry) -> Result<()> {
        self.ensure_log_file_initialized().await?;

        // Helper to escape tabs and newlines for TSV
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