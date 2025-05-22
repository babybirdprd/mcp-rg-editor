use crate::config::Config;
use crate::error::AppError;
use std::path::{Component, Path, PathBuf};
use tracing::debug;
// use std::sync::RwLockReadGuard; // No longer needed as argument type
use shellexpand;

/// Expands tilde (~) in a path string to the user's home directory.
pub fn expand_tilde_path_buf(path_str: &str) -> Result<PathBuf, AppError> {
    Ok(PathBuf::from(shellexpand::tilde(path_str).as_ref()))
}

/// Normalizes a path: expands tilde, makes it absolute relative to files_root if it's relative,
/// and then attempts to canonicalize it. Falls back to a simplified absolute path if canonicalization fails.
fn normalize_path_base(path_str: &str, files_root: &Path) -> Result<PathBuf, AppError> {
    let expanded_path = expand_tilde_path_buf(path_str)?;

    let absolute_path = if expanded_path.is_absolute() { // Corrected: removed mut
        expanded_path
    } else {
        files_root.join(expanded_path)
    };

    match dunce::canonicalize(&absolute_path) {
        Ok(canonical_path) => Ok(canonical_path),
        Err(_) => {
            let mut components_vec = Vec::new();
            for component in absolute_path.components() {
                match component {
                    Component::ParentDir => {
                        if let Some(Component::Normal(_)) = components_vec.last() {
                            components_vec.pop();
                        } else if cfg!(windows) && components_vec.len() == 1 {
                            if let Some(Component::Prefix(_)) = components_vec.first() {
                                // Allow C:\.. -> C:\
                            } else {
                                components_vec.push(component);
                            }
                        } else if cfg!(unix) && components_vec.is_empty() {
                            // Allow /.. -> /.. (or let it be handled by OS later)
                             components_vec.push(component);
                        }
                        // If ParentDir is at the start of a relative path, it's kept.
                         else if components_vec.is_empty() && !absolute_path.is_absolute() {
                            components_vec.push(component);
                        }
                    }
                    Component::CurDir => {}
                    _ => components_vec.push(component),
                }
            }
            Ok(components_vec.iter().collect())
        }
    }
}

pub fn validate_and_normalize_path(
    target_path_str: &str,
    config: &Config, // Changed from &RwLockReadGuard<Config> to &Config
    check_existence: bool,
    for_write_or_create: bool,
) -> Result<PathBuf, AppError> {
    debug!(target_path = %target_path_str, check_existence, for_write_or_create, "Validating path access");

    let normalized_target_path = normalize_path_base(target_path_str, &config.files_root)?;
    debug!(normalized_target_path = %normalized_target_path.display(), "Initial normalized target path");

    let path_for_dir_checks = if for_write_or_create && !normalized_target_path.exists() {
        normalized_target_path.parent().ok_or_else(|| AppError::InvalidPath(format!("Cannot determine parent directory for write/create: {}", normalized_target_path.display())))?.to_path_buf()
    } else {
        normalized_target_path.clone()
    };
    debug!(path_for_dir_checks = %path_for_dir_checks.display(), "Path used for directory/existence checks");

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

    let is_globally_allowed_by_config = config.allowed_directories.iter().any(|ad_config_path| {
        let normalized_ad = normalize_path_base(ad_config_path.to_str().unwrap_or(""), &config.files_root)
                                .unwrap_or_else(|_| ad_config_path.clone());
        normalized_ad == Path::new("/") || (cfg!(windows) && normalized_ad.parent().is_none() && normalized_ad.is_absolute())
    });

    if is_globally_allowed_by_config {
        debug!("Access globally allowed by an allowed_directory entry like '/' or 'C:\\'");
    } else {
        let is_specifically_allowed = config.allowed_directories.iter().any(|allowed_dir_config_entry| {
            let normalized_allowed_dir = normalize_path_base(allowed_dir_config_entry.to_str().unwrap_or_default(), &config.files_root)
                .unwrap_or_else(|_| allowed_dir_config_entry.clone());
            
            debug!(check_path = %path_for_dir_checks.display(), against_allowed_dir = %normalized_allowed_dir.display(), "Checking specific allowance");
            path_for_dir_checks.starts_with(&normalized_allowed_dir)
        });

        if !is_specifically_allowed {
            debug!(path = %normalized_target_path.display(), checked_against = %path_for_dir_checks.display(), allowed_dirs = ?config.allowed_directories, "Path not in allowed_directories");
            return Err(AppError::PathNotAllowed(format!(
                "Operation on path {} (effective check on {}) is not within any allowed directories. Allowed: {:?}",
                normalized_target_path.display(), path_for_dir_checks.display(), config.allowed_directories
            )));
        }
    }

    if check_existence {
        let path_to_check_existence = if for_write_or_create && !normalized_target_path.exists() {
            &path_for_dir_checks 
        } else {
            &normalized_target_path
        };

        if !path_to_check_existence.exists() {
            return Err(AppError::InvalidPath(format!(
                "Required path (or parent for write/create) does not exist: {}",
                path_to_check_existence.display()
            )));
        }
        if for_write_or_create && path_to_check_existence != &normalized_target_path && !path_to_check_existence.is_dir()  {
             return Err(AppError::InvalidPath(format!(
                "Parent path for write/create is not a directory: {}",
                path_to_check_existence.display()
            )));
        }
    }

    Ok(normalized_target_path)
}