//! Update command

use anyhow::{Context, Result, bail};
use reqwest::Client;
use apl::index::PackageIndex;
use apl::apl_home;

/// Update package index from CDN
pub async fn update(url: &str, dry_run: bool) -> Result<()> {
    let index_path = apl_home().join("index.bin");
    
    if dry_run {
        println!("Would download index from: {}", url);
        println!("Would save to: {}", index_path.display());
        return Ok(());
    }
    
    println!("🔄 Updating package index...");
    
    // Load current index for comparison
    let current_index = PackageIndex::load(&index_path).ok();
    
    let client = Client::new();
    let response = client.get(url).send().await
        .context("Failed to fetch index")?;
    
    if !response.status().is_success() {
        bail!("Failed to fetch index: HTTP {}", response.status());
    }
    
    let bytes = response.bytes().await?;
    
    // Decompress zstd, then parse postcard to check version/timestamp
    let decompressed = zstd::decode_all(bytes.as_ref())
        .context("Failed to decompress index")?;
    let index = PackageIndex::from_bytes(&decompressed)
        .context("Invalid index format")?;
    
    if let Some(current) = current_index {
        if current.updated_at == index.updated_at {
            println!("✓ Already up to date: {} packages", index.packages.len());
            return Ok(());
        }
    }
    
    // Save compressed data to disk (as-is from CDN)
    std::fs::write(&index_path, &bytes)?;
    
    println!("✓ Updated index: {} packages", index.packages.len());
    
    Ok(())
}
