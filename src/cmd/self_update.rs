//! Self-update command

use anyhow::{Context, Result, bail};
use apl::index::PackageIndex;
use apl::apl_home;

/// Update apl itself to the latest version
pub async fn self_update(dry_run: bool) -> Result<()> {
    let index_path = apl_home().join("index.bin");
    if !index_path.exists() {
        bail!("No index found. Run 'apl update' first.");
    }
    
    let index = PackageIndex::load(&index_path)
        .context("Failed to load index")?;
    
    let entry = index.find("apl")
        .context("apl package not found in index. Self-update not available.")?;
    
    // Get current version
    let current_version = env!("CARGO_PKG_VERSION");
    
    if entry.latest().version == current_version {
        println!("✓ apl is already at the latest version ({})", current_version);
        return Ok(());
    }
    
    println!("📦 Updating apl: {} → {}", current_version, entry.latest().version);
    
    if dry_run {
        return Ok(());
    }
    
    // Install the new version
    crate::cmd::install::install(&["apl".to_string()], false, false, false).await?;
    
    println!("✓ apl updated to {}", entry.latest().version);
    println!("  Restart your terminal to use the new version.");
    
    Ok(())
}
