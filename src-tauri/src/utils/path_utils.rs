// FILE: src-tauri/src/utils/path_utils.rs
use crate::config::Config;
use crate::error::AppError;
use std::path::{Component, Path, PathBuf};
// Removed anyhow::Result as AppError is used directly
use tracing::debug;
use std::sync::RwLockReadGuard;

/// Expands tilde (~) in a path string to the user's home directory.
pub fn expand_tilde_path_buf(path_str: &str) -> Result<PathBuf, AppError> {
    shellexpand::tilde(path_str)
        .map_err(|e| AppError::InvalidPath(format!("Tilde expansion failed for '{}': {}", path_str, e)))
        .map(|cow| PathBuf::from(cow.as_ref()))
}

/// Normalizes a path: expands tilde, makes it absolute relative to files_root if it's relative,
/// and then attempts to canonicalize it. Falls back to a simplified absolute path if canonicalization fails.
fn normalize_path_base(path_str: &str, files_root: &Path) -> Result<PathBuf, AppError> {
    let expanded_path = expand_tilde_path_buf(path_str)?;

    let mut absolute_path = if expanded_path.is_absolute() {
        expanded_path
    } else {
        files_root.join(expanded_path)
    };

    // Attempt to canonicalize. If it fails (e.g. path doesn't exist), use the constructed absolute path
    // after simplifying ".." and "." components.
    match dunce::canonicalize(&absolute_path) {
        Ok(canonical_path) => Ok(canonical_path),
        Err(_) => {
            let mut components_vec = Vec::new();
            for component in absolute_path.components() {
                match component {
                    Component::ParentDir => {
                        // Only pop if the last component was Normal and not the root/prefix.
                        if let Some(Component::Normal(_)) = components_vec.last() {
                            components_vec.pop();
                        } else if cfg!(windows) && components_vec.len() == 1 {
                            if let Some(Component::Prefix(_)) = components_vec.first() {
                                // Allow C:\.. -> C:\, don't pop prefix
                            } else {
                                // This case (e.g. relative path starting with .. from files_root)
                                // should be caught by starts_with(files_root) later.
                                // For now, if it's not a Normal component, we might be trying to go above root.
                                // Let's push it and let subsequent checks handle.
                                components_vec.push(component);
                            }
                        } else if cfg!(unix) && components_vec.is_empty() {
                             // Allow /../ -> /, push it and let subsequent checks handle.
                            components_vec.push(component);
                        }
                        // Otherwise, if it's already at root or trying to go above, do nothing or push.
                        // This logic aims to prevent `../../` from escaping a non-root `files_root`.
                    }
                    Component::CurDir => {} // Skip "."
                    _ => components_vec.push(component),
                }
            }
            Ok(components_vec.iter().collect())
        }
    }
}

/// Validates if a path is accessible based on FILES_ROOT and ALLOWED_DIRECTORIES.
/// `check_existence`: if true, the final path (or its parent for write/create) must exist.
/// `for_write_or_create`: if true, validates the parent directory for write/create operations.
pub fn validate_and_normalize_path(
    target_path_str: &str,
    config_guard: &RwLockReadGuard<Config>,
    check_existence: bool,
    for_write_or_create: bool,
) -> Result<PathBuf, AppError> {
    debug!(target_path = %target_path_str, check_existence, for_write_or_create, "Validating path access");

    let normalized_target_path = normalize_path_base(target_path_str, &config_guard.files_root)?;
    debug!(normalized_target_path = %normalized_target_path.display(), "Initial normalized target path");

    // Determine the path to check for directory-level permissions and existence
    let path_for_dir_checks = if for_write_or_create && !normalized_target_path.exists() {
        // If target for write/create doesn't exist, check its parent.
        normalized_target_path.parent().ok_or_else(|| AppError::InvalidPath(format!("Cannot determine parent directory for write/create: {}", normalized_target_path.display())))?.to_path_buf()
    } else {
        // Otherwise, check the target path itself (e.g., for read, or if target exists for write/create)
        normalized_target_path.clone()
    };
    debug!(path_for_dir_checks = %path_for_dir_checks.display(), "Path used for directory/existence checks");


    // 1. Check against FILES_ROOT
    let is_files_root_broad = config_guard.files_root == Path::new("/") ||
                              (cfg!(windows) && config_guard.files_root.parent().is_none() && config_guard.files_root.is_absolute());

    // The normalized_target_path should always be absolute or effectively absolute after normalize_path_base.
    // We need to ensure it's a subpath of files_root unless files_root is very broad.
    if !is_files_root_broad && !normalized_target_path.starts_with(&config_guard.files_root) {
        debug!(path = %normalized_target_path.display(), root = %config_guard.files_root.display(), "Path is outside files_root");
        return Err(AppError::PathTraversal(format!(
            "Path {} is outside of the configured root directory {}",
            normalized_target_path.display(),
            config_guard.files_root.display()
        )));
    }

    // 2. Check against ALLOWED_DIRECTORIES
    let is_globally_allowed_by_config = config_guard.allowed_directories.iter().any(|ad_config_path| {
        let normalized_ad = normalize_path_base(ad_config_path.to_str().unwrap_or(""), &config_guard.files_root)
                                .unwrap_or_else(|_| ad_config_path.clone()); // Fallback if normalization fails during check
        normalized_ad == Path::new("/") || (cfg!(windows) && normalized_ad.parent().is_none() && normalized_ad.is_absolute())
    });

    if is_globally_allowed_by_config {
        debug!("Access globally allowed by an allowed_directory entry like '/' or 'C:\\'");
    } else {
        // If not globally allowed, check if the path_for_dir_checks is within any of the specific allowed_directories.
        let is_specifically_allowed = config_guard.allowed_directories.iter().any(|allowed_dir_config_entry| {
            let normalized_allowed_dir = normalize_path_base(allowed_dir_config_entry.to_str().unwrap_or_default(), &config_guard.files_root)
                .unwrap_or_else(|_| allowed_dir_config_entry.clone());
            
            debug!(check_path = %path_for_dir_checks.display(), against_allowed_dir = %normalized_allowed_dir.display(), "Checking specific allowance");
            path_for_dir_checks.starts_with(&normalized_allowed_dir)
        });

        if !is_specifically_allowed {
            debug!(path = %normalized_target_path.display(), checked_against = %path_for_dir_checks.display(), allowed_dirs = ?config_guard.allowed_directories, "Path not in allowed_directories");
            return Err(AppError::PathNotAllowed(format!(
                "Operation on path {} (effective check on {}) is not within any allowed directories. Allowed: {:?}",
                normalized_target_path.display(), path_for_dir_checks.display(), config_guard.allowed_directories
            )));
        }
    }

    // 3. Check existence if required
    if check_existence {
        let path_to_check_existence = if for_write_or_create && !normalized_target_path.exists() {
            // For write/create, if target doesn't exist, we check existence of its parent (path_for_dir_checks).
            &path_for_dir_checks 
        } else {
            // For read, or if target exists for write/create, check target itself.
            &normalized_target_path
        };

        if !path_to_check_existence.exists() {
            return Err(AppError::InvalidPath(format!(
                "Required path (or parent for write/create) does not exist: {}",
                path_to_check_existence.display()
            )));
        }
        // If it's for_write_or_create and we checked the parent, it must be a directory.
        if for_write_or_create && path_to_check_existence != &normalized_target_path && !path_to_check_existence.is_dir()  {
             return Err(AppError::InvalidPath(format!(
                "Parent path for write/create is not a directory: {}",
                path_to_check_existence.display()
            )));
        }
    }

    Ok(normalized_target_path)
}