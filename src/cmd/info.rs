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
        let latest = entry.latest();
        println!("  Latest Version: {}", latest.version);
        if !entry.description.is_empty() {
            println!("  Description: {}", entry.description);
        }
        
        // Show versions
        let version_list: Vec<String> = entry.releases.iter().map(|r| r.version.clone()).collect();
        println!("  Available Versions: {}", version_list.join(", "));

        if !latest.deps.is_empty() {
            println!("  Dependencies: {}", latest.deps.join(", "));
        }
        if !latest.bin.is_empty() {
            println!("  Binaries: {}", latest.bin.join(", "));
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
