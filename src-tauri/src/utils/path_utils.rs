// FILE: src-tauri/src/utils/path_utils.rs
// IMPORTANT NOTE: Rewrite the entire file.
use crate::config::Config;
use crate::error::AppError;
use std::path::{Path, PathBuf};
use anyhow::Result;
use tracing::debug;
use std::sync::RwLockReadGuard;

/// Expands tilde (~) in a path string to the user's home directory.
pub fn expand_tilde_path_buf(path_str: &str) -> Result<PathBuf, AppError> {
    shellexpand::tilde(path_str)
        .map(|cow_str| PathBuf::from(cow_str.as_ref()))
        .map_err(|e| AppError::InvalidPath(format!("Failed to expand tilde for path '{}': {}", path_str, e)))
}

/// Normalizes a path: expands tilde, makes it absolute relative to files_root if it's relative,
/// and then attempts to canonicalize it. Falls back to the absolute path if canonicalization fails.
fn normalize_path_base(path_str: &str, files_root: &Path) -> Result<PathBuf, AppError> {
    let expanded_path = expand_tilde_path_buf(path_str)?;

    let mut absolute_path = if expanded_path.is_absolute() {
        expanded_path
    } else {
        files_root.join(expanded_path)
    };

    // Attempt to canonicalize. If it fails (e.g. path doesn't exist), use the constructed absolute path.
    // dunce::canonicalize is good for UNC paths on Windows.
    match dunce::canonicalize(&absolute_path) {
        Ok(canonical_path) => Ok(canonical_path),
        Err(_) => {
            // If canonicalization fails, it might be because the path (or parts of it) doesn't exist.
            // We still want to return an "absolute" form for validation against allowed_directories.
            // This simplified normalization removes ".." and "." components.
            let mut components = Vec::new();
            for component in absolute_path.components() {
                match component {
                    std::path::Component::ParentDir => {
                        if let Some(std::path::Component::Normal(_)) = components.last() {
                            components.pop();
                        } else if cfg!(unix) && components.is_empty() {
                            // e.g. /../ -> / (let it be, root check will handle)
                        } else if cfg!(windows) && components.len() == 1 && matches!(components.first(), Some(std::path::Component::Prefix(_))) {
                            // e.g. C:\.. -> C:\ (let it be)
                        } else {
                            // Path traversal attempt if trying to go above root or an empty path stack
                            // This case should ideally be caught by `starts_with(files_root)` later.
                            // For now, we just don't pop if it would go "above" the current component stack.
                        }
                    }
                    std::path::Component::CurDir => {} // Skip "."
                    _ => components.push(component),
                }
            }
            Ok(components.iter().collect())
        }
    }
}

/// Validates if a path is accessible based on FILES_ROOT and ALLOWED_DIRECTORIES.
/// `check_existence`: if true, the final path must exist.
/// `for_write_or_create`: if true, validates the parent directory for write/create operations,
///                        and the final path component's validity. The final path itself might not exist.
pub fn validate_and_normalize_path(
    target_path_str: &str,
    config_guard: &RwLockReadGuard<Config>,
    check_existence: bool,
    for_write_or_create: bool,
) -> Result<PathBuf, AppError> {
    debug!(target_path = %target_path_str, check_existence, for_write_or_create, "Validating path access");

    // Normalize the target path first.
    // If for_write_or_create, we normalize the full path to ensure the final component is valid,
    // but existence checks later will focus on the parent if the target doesn't exist.
    let normalized_target_path = normalize_path_base(target_path_str, &config_guard.files_root)?;
    debug!(normalized_target_path = %normalized_target_path.display(), "Initial normalized target path");

    // Determine the path to check for directory-level permissions and existence
    let path_for_dir_checks = if for_write_or_create && !normalized_target_path.exists() {
        normalized_target_path.parent().ok_or_else(|| AppError::InvalidPath(format!("Cannot determine parent directory for write/create: {}", normalized_target_path.display())))?.to_path_buf()
    } else {
        normalized_target_path.clone()
    };
    debug!(path_for_dir_checks = %path_for_dir_checks.display(), "Path used for directory/existence checks");


    // 1. Check against FILES_ROOT
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

    // 2. Check against ALLOWED_DIRECTORIES
    // An allowed_directory can be a broad root like "/" or "C:\"
    let is_globally_allowed_by_config = config_guard.allowed_directories.iter().any(|ad_config_path| {
        let normalized_ad = normalize_path_base(ad_config_path.to_str().unwrap_or(""), &config_guard.files_root)
                                .unwrap_or_else(|_| ad_config_path.clone());
        normalized_ad == Path::new("/") || (cfg!(windows) && normalized_ad.parent().is_none() && normalized_ad.is_absolute())
    });

    if is_globally_allowed_by_config {
        debug!("Access globally allowed by an allowed_directory entry like '/' or 'C:\\'");
    } else {
        // If not globally allowed, check if the path_for_dir_checks (or normalized_target_path if not for_write_or_create)
        // is within any of the specific allowed_directories.
        let path_to_check_against_allowed = if for_write_or_create && !normalized_target_path.is_dir() {
            // If writing/creating a file, check its parent dir against allowed_directories
            normalized_target_path.parent().unwrap_or(&normalized_target_path)
        } else {
            // If reading, or writing/creating a directory, check the path itself
            &normalized_target_path
        };


        let is_specifically_allowed = config_guard.allowed_directories.iter().any(|allowed_dir_config_entry| {
            let normalized_allowed_dir = normalize_path_base(allowed_dir_config_entry.to_str().unwrap_or_default(), &config_guard.files_root)
                .unwrap_or_else(|_| allowed_dir_config_entry.clone());
            debug!(path_to_check = %path_to_check_against_allowed.display(), allowed_dir_entry = %normalized_allowed_dir.display(), "Checking against allowed directory");
            path_to_check_against_allowed.starts_with(&normalized_allowed_dir)
        });

        if !is_specifically_allowed {
            debug!(path = %normalized_target_path.display(), allowed_dirs = ?config_guard.allowed_directories, "Path not in allowed_directories");
            return Err(AppError::PathNotAllowed(format!(
                "Operation on path {} (or its parent) is not within any allowed directories. Allowed: {:?}",
                normalized_target_path.display(), config_guard.allowed_directories
            )));
        }
    }

    // 3. Check existence if required
    if check_existence {
        // If for_write_or_create, we check existence of the parent dir if the target itself doesn't exist.
        // Otherwise, we check existence of the target path itself.
        let path_to_check_existence = if for_write_or_create && !normalized_target_path.exists() {
            path_for_dir_checks // This is already the parent
        } else {
            normalized_target_path.clone()
        };

        if !path_to_check_existence.exists() {
            return Err(AppError::InvalidPath(format!(
                "Required path (or parent for write/create) does not exist: {}",
                path_to_check_existence.display()
            )));
        }
        // If it's for_write_or_create and the parent exists, it must be a directory.
        if for_write_or_create && !path_to_check_existence.is_dir() && path_to_check_existence != normalized_target_path {
             return Err(AppError::InvalidPath(format!(
                "Parent path for write/create is not a directory: {}",
                path_to_check_existence.display()
            )));
        }
    }

    Ok(normalized_target_path)
}