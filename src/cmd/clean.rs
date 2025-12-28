//! Clean command (garbage collection)

use anyhow::{Context, Result};
use std::collections::HashSet;
use apl::db::StateDb;
use apl::cas_path;

/// Garbage collect orphaned CAS blobs
pub fn clean(dry_run: bool) -> Result<()> {
    let db = StateDb::open().context("Failed to open state database")?;
    let cas_dir = cas_path();
    
    if !cas_dir.exists() {
        println!("No cache directory found.");
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
                println!("Would remove: {}", path.display());
            } else {
                std::fs::remove_file(&path)?;
            }
        }
    }
    
    if orphan_count == 0 {
        println!("✓ No orphaned blobs found");
    } else if dry_run {
        println!("Would remove {} orphaned blobs ({} bytes)", orphan_count, orphan_bytes);
    } else {
        println!("✓ Removed {} orphaned blobs ({} bytes)", orphan_count, orphan_bytes);
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
