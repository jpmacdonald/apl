//! Clean command (garbage collection)

use anyhow::{Context, Result};
use apl::cas_path;
use apl::db::StateDb;
use std::collections::HashSet;

/// Garbage collect orphaned CAS blobs
pub fn clean(dry_run: bool) -> Result<()> {
    let db = StateDb::open().context("Failed to open state database")?;
    let cas_dir = cas_path();

    let output = apl::io::output::CliOutput::new();

    if !cas_dir.exists() {
        output.info("No cache directory found.");
        return Ok(());
    }

    // Get all hashes referenced in DB
    let mut referenced_hashes = HashSet::new();
    for pkg in db.list_packages()? {
        for file in db.get_package_files(&pkg.name)? {
            referenced_hashes.insert(file.blake3);
        }
    }

    // Walk CAS directory and find orphans
    let mut orphan_count = 0;
    let mut orphan_bytes = 0u64;

    for entry in walkdir(&cas_dir) {
        let path = entry;

        if !path.is_file() {
            continue;
        }

        // Extract hash from path (last component)
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");

        if !referenced_hashes.contains(file_name) {
            orphan_count += 1;
            orphan_bytes += path.metadata().map(|m| m.len()).unwrap_or(0);

            if dry_run {
                output.info(&format!("Would remove: {}", path.display()));
            } else {
                std::fs::remove_file(&path)?;
            }
        }
    }

    if orphan_count == 0 {
        output.success("No orphaned blobs found");
    } else if dry_run {
        output.info(&format!(
            "Would remove {orphan_count} orphaned blobs ({})",
            apl::io::output::format_size(orphan_bytes)
        ));
    } else {
        output.success(&format!(
            "Removed {orphan_count} orphaned blobs ({})",
            apl::io::output::format_size(orphan_bytes)
        ));
    }

    Ok(())
}

/// Simple recursive directory walker
fn walkdir(path: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut results = Vec::new();

    fn walk(path: &std::path::Path, results: &mut Vec<std::path::PathBuf>) {
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                let entry_path = entry.path();
                results.push(entry_path.clone());
                if entry_path.is_dir() {
                    walk(&entry_path, results);
                }
            }
        }
    }

    walk(path, &mut results);
    results
}
