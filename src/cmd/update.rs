//! Update command

use anyhow::{Context, Result, bail};
use reqwest::Client;
use dl::index::PackageIndex;
use dl::dl_home;

/// Update package index from CDN
pub async fn update(url: &str, dry_run: bool) -> Result<()> {
    let index_path = dl_home().join("index.bin");
    
    if dry_run {
        println!("Would download index from: {}", url);
        println!("Would save to: {}", index_path.display());
        return Ok(());
    }
    
    println!("🔄 Updating package index...");
    
    let client = Client::new();
    let response = client.get(url).send().await
        .context("Failed to fetch index")?;
    
    if !response.status().is_success() {
        bail!("Failed to fetch index: HTTP {}", response.status());
    }
    
    let bytes = response.bytes().await?;
    
    // Decompress zstd, then parse postcard
    let decompressed = zstd::decode_all(bytes.as_ref())
        .context("Failed to decompress index")?;
    let index = PackageIndex::from_bytes(&decompressed)
        .context("Invalid index format")?;
    
    // Save compressed data to disk (as-is from CDN)
    std::fs::write(&index_path, &bytes)?;
    
    println!("✓ Updated index: {} packages", index.packages.len());
    
    Ok(())
}
