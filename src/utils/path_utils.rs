use crate::config::Config;
use crate::error::AppError;
use std::path::{Component, Path, PathBuf};
use anyhow::Result;
use tracing::debug;
use std::sync::RwLockReadGuard; // For reading from RwLock<Config>

/// Expands tilde (~) in a path string to the user's home directory.
pub fn expand_tilde_path_buf(path_str: &str) -> Result<PathBuf, AppError> {
    shellexpand::tilde(path_str)
        .map(|cow_str| PathBuf::from(cow_str.as_ref())) // Use .as_ref()
        .map_err(|e| AppError::InvalidPath(format!("Failed to expand tilde for path '{}': {}", path_str, e)))
}

fn normalize_path(path_str: &str, files_root: &Path) -> Result<PathBuf, AppError> {
    let expanded_path = expand_tilde_path_buf(path_str)?;

    let mut absolute_path = if expanded_path.is_absolute() {
        expanded_path
    } else {
        files_root.join(expanded_path)
    };

    let mut components = Vec::new();
    for component in absolute_path.components() {
        match component {
            Component::ParentDir => {
                if let Some(Component::Normal(_)) = components.last() {
                    components.pop();
                } else if cfg!(unix) && components.is_empty() {
                    // Allow, e.g. /../ -> /
                } else if cfg!(windows) && components.len() == 1 && matches!(components.first(), Some(Component::Prefix(_))) {
                    // Allow, e.g. C:\.. -> C:\
                } else {
                    // Path traversal attempt if trying to go above root or an empty path stack
                    // For now, let canonicalize handle or allow if it's a valid operation within allowed root
                }
            }
            _ => components.push(component),
        }
    }
    absolute_path = components.iter().collect();

    match dunce::canonicalize(&absolute_path) { // Using dunce for better Windows UNC/prefix handling
        Ok(canonical_path) => Ok(canonical_path),
        Err(_) => Ok(absolute_path), 
    }
}


pub fn validate_path_access(
    target_path_str: &str,
    config_guard: &RwLockReadGuard<Config>, // Pass read guard
    check_existence: bool,
) -> Result<PathBuf, AppError> {
    debug!(target_path = %target_path_str, check_existence = %check_existence, "Validating path access");
    let normalized_target_path = normalize_path(target_path_str, &config_guard.files_root)?;
    debug!(normalized_path = %normalized_target_path.display(), "Normalized path");

    if check_existence && !normalized_target_path.exists() {
        return Err(AppError::InvalidPath(format!(
            "Path does not exist: {}",
            normalized_target_path.display()
        )));
    }
    
    let is_files_root_broad = config_guard.files_root == Path::new("/") ||
                              (cfg!(windows) && config_guard.files_root.parent().is_none() && config_guard.files_root.is_absolute());

    if !is_files_root_broad && !normalized_target_path.starts_with(&config_guard.files_root) {
         debug!(path = %normalized_target_path.display(), root = %config_guard.files_root.display(), "Path is outside files_root");
        return Err(AppError::PathTraversal(format!(
            "Path {} is outside of the configured root directory {}",
            normalized_target_path.display(),
            config_guard.files_root.display()
        )));
    }

    let is_globally_allowed = config_guard.allowed_directories.iter().any(|ad| {
        let normalized_ad = normalize_path(ad.to_str().unwrap_or(""), &config_guard.files_root).unwrap_or_else(|_| ad.clone());
        normalized_ad == Path::new("/") || (cfg!(windows) && normalized_ad.parent().is_none() && normalized_ad.is_absolute())
    });

    if is_globally_allowed {
        debug!("Access globally allowed by an allowed_directory entry like '/' or 'C:\\'");
        return Ok(normalized_target_path);
    }
    
    let is_specifically_allowed = config_guard.allowed_directories.iter().any(|allowed_dir_config_entry| {
        let normalized_allowed_dir = normalize_path(allowed_dir_config_entry.to_str().unwrap_or_default(), &config_guard.files_root)
            .unwrap_or_else(|_| allowed_dir_config_entry.clone());
        
        debug!(path_to_check = %normalized_target_path.display(), allowed_dir_entry = %normalized_allowed_dir.display(), "Checking against allowed directory");
        normalized_target_path.starts_with(&normalized_allowed_dir)
    });

    if !is_specifically_allowed {
        debug!(path = %normalized_target_path.display(), allowed_dirs = ?config_guard.allowed_directories, "Path not in allowed_directories");
        return Err(AppError::PathNotAllowed(format!(
            "Path {} is not within any allowed directories. Allowed: {:?}",
            normalized_target_path.display(), config_guard.allowed_directories
        )));
    }

    Ok(normalized_target_path)
}

pub fn validate_parent_path_access(
    target_path_str: &str,
    config_guard: &RwLockReadGuard<Config>, // Pass read guard
) -> Result<PathBuf, AppError> {
    debug!(target_path = %target_path_str, "Validating parent path access");
    let normalized_target_path = normalize_path(target_path_str, &config_guard.files_root)?;
    debug!(normalized_path = %normalized_target_path.display(), "Normalized path for parent validation");

    match normalized_target_path.parent() {
        Some(parent) => {
            validate_path_access(parent.to_str().unwrap_or_default(), config_guard, true)?;
            Ok(normalized_target_path)
        }
        None => {
            // If there's no parent, it means it's a root-like path (e.g., "/" or "C:\").
            // Validate this path itself.
            validate_path_access(normalized_target_path.to_str().unwrap_or_default(), config_guard, false)?; // check_existence false for root itself
            Ok(normalized_target_path)
        }
    }
}