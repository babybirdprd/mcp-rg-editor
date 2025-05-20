use crate::config::Config;
use crate::error::AppError;
use std::path::{Component, Path, PathBuf};
use anyhow::Result; // Using anyhow's Result for internal operations here
use tracing::debug;

/// Expands tilde (~) in a path string to the user's home directory.
pub fn expand_tilde_path_buf(path_str: &str) -> Result<PathBuf, AppError> {
    shellexpand::tilde(path_str)
        .map(PathBuf::from)
        .map_err(|e| AppError::InvalidPath(format!("Failed to expand tilde for path '{}': {}", path_str, e)))
}

/// Normalizes a path: expands tilde, makes absolute, canonicalizes if exists.
/// If the path doesn't exist, it normalizes up to the closest existing parent.
fn normalize_path(path_str: &str, files_root: &Path) -> Result<PathBuf, AppError> {
    let expanded_path = expand_tilde_path_buf(path_str)?;

    let mut absolute_path = if expanded_path.is_absolute() {
        expanded_path
    } else {
        files_root.join(expanded_path)
    };

    // Normalize ".." components manually to prevent escaping root if path doesn't exist yet.
    // This is a simplified normalization. For robust ".." handling before canonicalize,
    // one might need a more complex path resolution logic.
    let mut components = Vec::new();
    for component in absolute_path.components() {
        match component {
            Component::ParentDir => {
                if let Some(Component::Normal(_)) = components.last() {
                    components.pop();
                } else if cfg!(unix) && components.is_empty() {
                    // Trying to `../` from root on Unix, keep it as is for now
                    // or error depending on strictness. For now, let canonicalize handle.
                } else if cfg!(windows) && components.len() == 1 && matches!(components.first(), Some(Component::Prefix(_))) {
                    // Trying to `../` from drive root on Windows
                }
            }
            _ => components.push(component),
        }
    }
    absolute_path = components.iter().collect();


    // Attempt to canonicalize. If it fails (e.g. path doesn't exist),
    // work with the constructed absolute path.
    match absolute_path.canonicalize() {
        Ok(canonical_path) => Ok(canonical_path),
        Err(_) => Ok(absolute_path), // Path might not exist yet (e.g. for write_file, create_directory)
    }
}


/// Validates if the given `target_path_str` is within the `files_root`
/// and one of the `allowed_directories`.
/// Returns the normalized, absolute path if valid.
/// `check_existence` flag determines if the path itself must exist.
pub fn validate_path_access(
    target_path_str: &str,
    config: &Config,
    check_existence: bool,
) -> Result<PathBuf, AppError> {
    debug!(target_path = %target_path_str, check_existence = %check_existence, "Validating path access");
    let normalized_target_path = normalize_path(target_path_str, &config.files_root)?;
    debug!(normalized_path = %normalized_target_path.display(), "Normalized path");

    if check_existence && !normalized_target_path.exists() {
        return Err(AppError::InvalidPath(format!(
            "Path does not exist: {}",
            normalized_target_path.display()
        )));
    }

    // Check 1: Is it under files_root?
    // (Unless files_root is itself a broad root like "/" or "C:\")
    let is_files_root_broad = config.files_root == Path::new("/") ||
                              (cfg!(windows) && config.files_root.parent().is_none() && config.files_root.is_absolute());

    if !is_files_root_broad && !normalized_target_path.starts_with(&config.files_root) {
         debug!(path = %normalized_target_path.display(), root = %config.files_root.display(), "Path is outside files_root");
        return Err(AppError::PathTraversal(format!(
            "Path {} is outside of the configured root directory {}",
            normalized_target_path.display(),
            config.files_root.display()
        )));
    }

    // Check 2: Is it within any of the allowed_directories?
    // An allowed directory of "/" or "C:\" means all paths are allowed (already covered by files_root if it's the same)
    let is_globally_allowed = config.allowed_directories.iter().any(|ad| {
        ad == Path::new("/") || (cfg!(windows) && ad.parent().is_none() && ad.is_absolute())
    });

    if is_globally_allowed {
        debug!("Access globally allowed by an allowed_directory entry like '/' or 'C:\\'");
        return Ok(normalized_target_path);
    }
    
    let is_specifically_allowed = config.allowed_directories.iter().any(|allowed_dir_config_entry| {
        // Normalize the allowed_dir_config_entry from config for comparison
        // It might be relative to files_root or absolute.
        let normalized_allowed_dir = normalize_path(allowed_dir_config_entry.to_str().unwrap_or_default(), &config.files_root)
            .unwrap_or_else(|_| allowed_dir_config_entry.clone()); // Fallback if normalization fails (e.g. it's already absolute and fine)
        
        debug!(path_to_check = %normalized_target_path.display(), allowed_dir_entry = %normalized_allowed_dir.display(), "Checking against allowed directory");
        normalized_target_path.starts_with(&normalized_allowed_dir)
    });

    if !is_specifically_allowed {
        debug!(path = %normalized_target_path.display(), allowed_dirs = ?config.allowed_directories, "Path not in allowed_directories");
        return Err(AppError::PathNotAllowed(format!(
            "Path {} is not within any allowed directories. Allowed: {:?}",
            normalized_target_path.display(), config.allowed_directories
        )));
    }

    Ok(normalized_target_path)
}

/// Validates if the PARENT of `target_path_str` is accessible.
/// Used for operations that create new files/directories.
pub fn validate_parent_path_access(
    target_path_str: &str,
    config: &Config,
) -> Result<PathBuf, AppError> {
    debug!(target_path = %target_path_str, "Validating parent path access");
    let normalized_target_path = normalize_path(target_path_str, &config.files_root)?;
    debug!(normalized_path = %normalized_target_path.display(), "Normalized path for parent validation");

    match normalized_target_path.parent() {
        Some(parent) => {
            // Validate the parent directory itself. It must exist.
            validate_path_access(parent.to_str().unwrap_or_default(), config, true)?;
            Ok(normalized_target_path) // Return the original normalized target path
        }
        None => {
            // This means target_path_str is likely a root path like "/" or "C:\"
            // In this case, we validate the root path itself.
            validate_path_access(normalized_target_path.to_str().unwrap_or_default(), config, true)?;
            Ok(normalized_target_path)
        }
    }
}