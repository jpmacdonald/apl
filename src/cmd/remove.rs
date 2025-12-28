//! Remove command

use anyhow::{Context, Result, bail};
use dl::db::StateDb;

/// Remove one or more packages
pub fn remove(packages: &[String], dry_run: bool) -> Result<()> {
    let db = StateDb::open().context("Failed to open state database")?;
    
    for pkg in packages {
        // Get file list before removing
        let files = db.get_package_files(pkg)?;
        
        if files.is_empty() {
            bail!("Package '{}' is not installed", pkg);
        }
        
        if dry_run {
            println!("Would remove: {}", pkg);
            for file in &files {
                println!("  Would delete: {}", file.path);
            }
            continue;
        }
        
        // Delete files
        for file in &files {
            if let Err(e) = std::fs::remove_file(&file.path) {
                // Only warn, don't fail
                eprintln!("  Warning: could not remove {}: {}", file.path, e);
            }
        }
        
        // Remove from DB
        db.remove_package(pkg)?;
        
        println!("✓ {} removed ({} files)", pkg, files.len());
    }
    
    Ok(())
}
