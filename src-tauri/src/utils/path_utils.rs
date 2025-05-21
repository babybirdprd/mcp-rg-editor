use crate::config::Config;
use crate::error::AppError;
use std::path::{Component, Path, PathBuf};
use anyhow::Result;
use tracing::debug;
use std::sync::RwLockReadGuard;

/// Expands tilde (~) in a path string to the user's home directory.
pub fn expand_tilde_path_buf(path_str: &str) -> Result<PathBuf, AppError> {
    shellexpand::tilde(path_str)
        .map(|cow_str| PathBuf::from(cow_str.as_ref()))
        .map_err(|e| AppError::InvalidPath(format!("Failed to expand tilde for path '{}': {}", path_str, e)))
}

fn normalize_path_within_root(path_str: &str, files_root: &Path) -> Result<PathBuf, AppError> {
    let expanded_path = expand_tilde_path_buf(path_str)?;

    let mut absolute_path = if expanded_path.is_absolute() {
        expanded_path
    } else {
        // If relative, join with files_root FIRST, then attempt canonicalization
        files_root.join(expanded_path)
    };

    // Attempt to canonicalize. If it fails (e.g. path doesn't exist), use the constructed absolute path.
    // dunce::canonicalize is good for UNC paths on Windows.
    let canonical_path = dunce::canonicalize(&absolute_path).unwrap_or_else(|_| absolute_path.clone());

    // Final check: ensure no ".." components escape the files_root after all normalization.
    // This is a simplified check; a more robust one would iterate through components.
    if !canonical_path.starts_with(files_root) && files_root != Path::new("/") && !(cfg!(windows) && files_root.parent().is_none() && files_root.is_absolute()) {
         // Allow if files_root is "/" or a drive root like "C:\"
        if !(files_root == Path::new("/") || (cfg!(windows) && files_root.parent().is_none() && files_root.is_absolute())) {
             debug!(normalized_path = %canonical_path.display(), root = %files_root.display(), "Path normalization resulted in path outside root");
            // return Err(AppError::PathTraversal(format!(
            //     "Normalized path {} is outside of the files_root {}",
            //     canonical_path.display(),
            //     files_root.display()
            // )));
            // For now, we let the validate_path_access handle this specific check more granularly with allowed_directories.
        }
    }
    Ok(canonical_path)
}


// Validates if a path is accessible based on FILES_ROOT and ALLOWED_DIRECTORIES
// check_existence: if true, the path must exist.
// for_write: if true, checks parent for write operations.
pub fn validate_and_normalize_path(
    target_path_str: &str,
    config_guard: &RwLockReadGuard<Config>,
    check_existence: bool,
    for_write: bool, // If true, we validate the parent directory for write/create operations
) -> Result<PathBuf, AppError> {
    debug!(target_path = %target_path_str, check_existence, for_write, "Validating path access");

    let path_to_validate_str = if for_write {
        // For writes, we are interested in the parent directory's accessibility
        // and the final path component's validity.
        // The final path itself might not exist yet.
        let temp_path = expand_tilde_path_buf(target_path_str)?;
        if let Some(parent) = temp_path.parent() {
            parent.to_str().unwrap_or("").to_string()
        } else {
            // If no parent (e.g. "file.txt" in root, or "/"), validate target_path_str itself.
            target_path_str.to_string()
        }
    } else {
        target_path_str.to_string()
    };

    let normalized_base_path = normalize_path_within_root(&path_to_validate_str, &config_guard.files_root)?;
    let final_normalized_path = if for_write {
        // Re-attach the filename if we validated the parent
        let filename = PathBuf::from(target_path_str).file_name().ok_or_else(|| AppError::InvalidPath("Path has no filename component for write".to_string()))?;
        normalized_base_path.join(filename)
    } else {
        normalized_base_path.clone() // Use clone of normalized_base_path
    };


    debug!(normalized_path = %final_normalized_path.display(), "Normalized path for validation");

    // 1. Check against FILES_ROOT
    let is_files_root_broad = config_guard.files_root == Path::new("/") ||
                              (cfg!(windows) && config_guard.files_root.parent().is_none() && config_guard.files_root.is_absolute());

    if !is_files_root_broad && !final_normalized_path.starts_with(&config_guard.files_root) {
        debug!(path = %final_normalized_path.display(), root = %config_guard.files_root.display(), "Path is outside files_root");
        return Err(AppError::PathTraversal(format!(
            "Path {} is outside of the configured root directory {}",
            final_normalized_path.display(),
            config_guard.files_root.display()
        )));
    }

    // 2. Check against ALLOWED_DIRECTORIES
    let is_globally_allowed_by_config = config_guard.allowed_directories.iter().any(|ad_config_path| {
        let normalized_ad = normalize_path_within_root(ad_config_path.to_str().unwrap_or(""), &config_guard.files_root)
                                .unwrap_or_else(|_| ad_config_path.clone()); // Use original if normalization fails
        normalized_ad == Path::new("/") || (cfg!(windows) && normalized_ad.parent().is_none() && normalized_ad.is_absolute())
    });

    if is_globally_allowed_by_config {
        debug!("Access globally allowed by an allowed_directory entry like '/' or 'C:\\'");
    } else {
        let is_specifically_allowed = config_guard.allowed_directories.iter().any(|allowed_dir_config_entry| {
            let normalized_allowed_dir = normalize_path_within_root(allowed_dir_config_entry.to_str().unwrap_or_default(), &config_guard.files_root)
                .unwrap_or_else(|_| allowed_dir_config_entry.clone());
            debug!(path_to_check = %final_normalized_path.display(), allowed_dir_entry = %normalized_allowed_dir.display(), "Checking against allowed directory");
            final_normalized_path.starts_with(&normalized_allowed_dir)
        });

        if !is_specifically_allowed {
            debug!(path = %final_normalized_path.display(), allowed_dirs = ?config_guard.allowed_directories, "Path not in allowed_directories");
            return Err(AppError::PathNotAllowed(format!(
                "Path {} is not within any allowed directories. Allowed: {:?}",
                final_normalized_path.display(), config_guard.allowed_directories
            )));
        }
    }

    // 3. Check existence if required (and not for parent of a write op)
    if check_existence && !for_write && !final_normalized_path.exists() {
        return Err(AppError::InvalidPath(format!(
            "Path does not exist: {}",
            final_normalized_path.display()
        )));
    }
    // If for_write, the parent must exist (which is implicitly checked by normalize_path_within_root if it canonicalizes)
    // or if it doesn't canonicalize, the constructed path's parent should be valid.
    // The tauri-plugin-fs will handle actual file creation errors.

    Ok(final_normalized_path)
}