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
        
        // Get version for history
        let version_from = db.get_package(pkg).ok().flatten().map(|p| p.version);

        // Delete files
        for file in &files {
            let path = std::path::Path::new(&file.path);
            let result = if path.is_dir() {
                std::fs::remove_dir_all(path)
            } else {
                std::fs::remove_file(path)
            };
            
            if let Err(e) = result {
                // Only warn, don't fail
                eprintln!("  Warning: could not remove {}: {}", file.path, e);
            }
        }
        
        // Remove from DB
        db.remove_package(pkg)?;
        
        // Record history
        if let Some(v) = version_from {
            db.add_history(pkg, "remove", Some(&v), None, true)?;
        }

        println!("✓ {} removed ({} files)", pkg, files.len());
    }
    
    Ok(())
}
