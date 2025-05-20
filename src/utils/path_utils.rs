use crate::config::Config;
use crate::error::AppError;
use std::path::{Path, PathBuf};
use anyhow::Result;

/// Validates if the given `target_path` is within one of the `allowed_directories`
/// and also under the `files_root`.
/// Returns the canonicalized, absolute path if valid.
pub fn validate_path_access(
    target_path_str: &str,
    config: &Config,
    check_existence: bool,
) -> Result<PathBuf, AppError> {
    let mut target_path = PathBuf::from(target_path_str);

    // If path is relative, join it with files_root
    if target_path.is_relative() {
        target_path = config.files_root.join(target_path);
    }
    
    // Canonicalize the path to resolve symlinks and ".."
    let canonical_target_path = target_path
        .canonicalize()
        .map_err(|e| AppError::InvalidPath(format!("Failed to canonicalize path '{}': {}", target_path.display(), e)))?;

    if check_existence && !canonical_target_path.exists() {
         return Err(AppError::InvalidPath(format!("Path does not exist: {}", canonical_target_path.display())));
    }

    // Check if it's under files_root
    if !canonical_target_path.starts_with(&config.files_root) {
        return Err(AppError::PathTraversal(format!(
            "Path {} is outside of root {}",
            canonical_target_path.display(),
            config.files_root.display()
        )));
    }

    // Check if it's within any of the allowed_directories
    let is_allowed = config.allowed_directories.iter().any(|allowed_dir| {
        canonical_target_path.starts_with(allowed_dir)
    });

    if !is_allowed {
        return Err(AppError::PathNotAllowed(format!(
            "Path {} is not in allowed directories",
            canonical_target_path.display()
        )));
    }

    Ok(canonical_target_path)
}


// Helper for operations that create new files/dirs, where the final path might not exist yet,
// but its parent must be valid.
pub fn validate_parent_path_access(
    target_path_str: &str,
    config: &Config,
) -> Result<PathBuf, AppError> {
    let mut target_path = PathBuf::from(target_path_str);

    if target_path.is_relative() {
        target_path = config.files_root.join(target_path);
    }

    // We don't canonicalize target_path itself as it might not exist.
    // Instead, we find the closest existing parent and canonicalize that.
    let mut current = target_path.clone();
    let mut existing_parent = None;
    while let Some(parent) = current.parent() {
        if parent.exists() {
            existing_parent = Some(parent.to_path_buf());
            break;
        }
        current = parent.to_path_buf();
        if parent == Path::new("") || parent == Path::new("/") { // Reached root
            break;
        }
    }
    
    let parent_to_validate = existing_parent.unwrap_or_else(|| config.files_root.clone());

    let canonical_parent = parent_to_validate
        .canonicalize()
        .map_err(|e| AppError::InvalidPath(format!("Failed to canonicalize parent path '{}': {}", parent_to_validate.display(), e)))?;
    
    // Ensure the canonical parent is within files_root and allowed_directories
    if !canonical_parent.starts_with(&config.files_root) {
        return Err(AppError::PathTraversal(format!(
            "Parent path {} is outside of root {}",
            canonical_parent.display(),
            config.files_root.display()
        )));
    }
    let is_parent_allowed = config.allowed_directories.iter().any(|allowed_dir| {
        canonical_parent.starts_with(allowed_dir)
    });
     if !is_parent_allowed {
        return Err(AppError::PathNotAllowed(format!(
            "Parent path {} is not in allowed directories",
            canonical_parent.display()
        )));
    }

    // Construct the final absolute path using the validated parent.
    // This helps prevent issues if `target_path_str` contained `..` that would go above `canonical_parent`
    // if naively joined.
    // Example: target_path_str = "existing_parent/../new_dir"
    // We need to ensure `new_dir` is created relative to `canonical_parent`'s actual location.
    // However, PathBuf::join handles this correctly by normalizing.
    // The main check is that `canonical_parent` itself is allowed.

    // The final path is simply the absolute version of the original target_path,
    // as its parentage has been validated.
    Ok(target_path.as_path().to_path_buf())
}