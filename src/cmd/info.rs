//! Info command

use anyhow::{Context, Result, bail};
use dl::db::StateDb;
use dl::index::PackageIndex;
use dl::dl_home;

/// Show info about a specific package
pub fn info(package: &str) -> Result<()> {
    let db = StateDb::open().context("Failed to open state database")?;
    
    // Check if installed
    let installed = db.get_package(package)?;
    
    // Check index for more info
    let index_path = dl_home().join("index.bin");
    let index_entry = if index_path.exists() {
        PackageIndex::load(&index_path).ok().and_then(|idx| idx.find(package).cloned())
    } else {
        None
    };
    
    if installed.is_none() && index_entry.is_none() {
        bail!("Package '{}' not found", package);
    }
    
    println!("📦 {}", package);
    
    if let Some(entry) = &index_entry {
        println!("  Version: {}", entry.version);
        if !entry.description.is_empty() {
            println!("  Description: {}", entry.description);
        }
        if !entry.deps.is_empty() {
            println!("  Dependencies: {}", entry.deps.join(", "));
        }
        if !entry.bin.is_empty() {
            println!("  Binaries: {}", entry.bin.join(", "));
        }
    }
    
    if let Some(pkg) = &installed {
        println!("  Status: Installed ({})", pkg.version);
        let files = db.get_package_files(package)?;
        if !files.is_empty() {
            println!("  Files:");
            for file in files {
                println!("    {}", file.path);
            }
        }
    } else {
        println!("  Status: Not installed");
    }
    
    Ok(())
}
