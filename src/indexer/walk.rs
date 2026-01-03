//! Registry directory traversal utilities.

use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};

/// Walk a registry directory and return all TOML template files.
///
/// Handles both sharded (registry/ab/abc.toml) and flat (registry/abc.toml) layouts.
pub fn walk_registry_toml_files(registry_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut toml_files = Vec::new();

    // Check for sharded layout (directories like "1", "aa", "ab", etc.)
    let is_sharded = registry_dir.join("1").exists()
        || fs::read_dir(registry_dir)?.filter_map(|e| e.ok()).any(|e| {
            let path = e.path();
            path.is_dir() && path.file_name().is_some_and(|n| n.len() == 2)
        });

    if is_sharded {
        // Sharded layout: registry/{prefix}/{name}.toml
        for entry in fs::read_dir(registry_dir)? {
            let entry = entry?;
            let prefix_path = entry.path();

            if !prefix_path.is_dir() {
                continue;
            }

            for sub_entry in fs::read_dir(&prefix_path)? {
                let sub_entry = sub_entry?;
                let path = sub_entry.path();
                if path.extension().is_some_and(|e| e == "toml") {
                    toml_files.push(path);
                }
            }
        }
    } else {
        // Flat layout: registry/{name}.toml
        for entry in fs::read_dir(registry_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "toml") {
                toml_files.push(path);
            }
        }
    }

    Ok(toml_files)
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
