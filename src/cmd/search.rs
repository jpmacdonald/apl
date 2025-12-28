//! Search command

use anyhow::{Context, Result, bail};
use apl::index::PackageIndex;
use apl::apl_home;

/// Search packages in the local index
pub fn search(query: &str) -> Result<()> {
    let index_path = apl_home().join("index.bin");
    if !index_path.exists() {
        bail!("No index found. Run 'dl update' first.");
    }
    
    let index = PackageIndex::load(&index_path)
        .context("Failed to load index")?;
    
    let results = index.search(query);
    
    if results.is_empty() {
        println!("No packages found matching '{}'", query);
        return Ok(());
    }
    
    println!("📦 Packages matching '{}':", query);
    for entry in results {
        println!("  {} {} — {}", entry.name, entry.latest().version, entry.description);
    }
    
    Ok(())
}
