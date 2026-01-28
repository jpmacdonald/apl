//! Registry directory traversal utilities.

use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

/// Walk a registry directory and return all TOML template files.
///
/// Handles both sharded (`registry/ab/abc.toml`) and flat
/// (`registry/abc.toml`) layouts.
///
/// # Errors
///
/// Returns an error if the registry directory cannot be read.
pub fn walk_registry_toml_files(registry_dir: &Path) -> Result<Box<dyn Iterator<Item = PathBuf>>> {
    // Check for sharded layout (directories like "1", "aa", "ab", etc.)
    let is_sharded = registry_dir.join("1").exists()
        || fs::read_dir(registry_dir)?
            .filter_map(std::result::Result::ok)
            .any(|e| {
                let path = e.path();
                path.is_dir() && path.file_name().is_some_and(|n| n.len() == 2)
            });

    if is_sharded {
        let registry_dir = registry_dir.to_path_buf();
        let iter = fs::read_dir(registry_dir)?
            .filter_map(std::result::Result::ok)
            .filter(|e| e.path().is_dir())
            .flat_map(|prefix_entry| fs::read_dir(prefix_entry.path()).ok().into_iter().flatten())
            .filter_map(std::result::Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|ext| ext == "toml"));
        Ok(Box::new(iter))
    } else {
        let iter = fs::read_dir(registry_dir)?
            .filter_map(std::result::Result::ok)
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|ext| ext == "toml"));
        Ok(Box::new(iter))
    }
}

/// Compute the sharded registry path for a package name.
///
/// - Single-letter names: `registry/1/{name}.toml`
/// - Multi-letter names: `registry/{first-two-letters}/{name}.toml`
pub fn registry_path(registry_dir: &Path, name: &str) -> PathBuf {
    let prefix = if name.len() == 1 {
        "1".to_string()
    } else {
        name[..2].to_lowercase()
    };
    registry_dir.join(prefix).join(format!("{name}.toml"))
}
