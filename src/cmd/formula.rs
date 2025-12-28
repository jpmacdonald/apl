//! Formula management commands

use anyhow::{Context, Result};
use std::path::Path;
use apl::formula::Formula;

/// Create a new formula template
pub fn new(name: &str, output_dir: &Path) -> Result<()> {
    let filename = format!("{}.toml", name);
    let path = output_dir.join(&filename);
    
    if path.exists() {
        anyhow::bail!("Formula already exists: {}", path.display());
    }
    
    let template = format!(r#"[package]
name = "{name}"
version = "0.1.0"
description = ""
homepage = ""

[source]
url = "https://github.com/OWNER/{name}/archive/refs/tags/v0.1.0.tar.gz"
blake3 = "PLACEHOLDER"

[bottle.arm64]
url = "https://example.com/releases/download/v0.1.0/{name}-0.1.0-arm64.tar.gz"
blake3 = "PLACEHOLDER"

[bottle.x86_64]
url = "https://example.com/releases/download/v0.1.0/{name}-0.1.0-x86_64.tar.gz"
blake3 = "PLACEHOLDER"

[install]
bin = ["{name}"]

[dependencies]
"#);
    
    std::fs::create_dir_all(output_dir)?;
    std::fs::write(&path, template)?;
    
    println!("✓ Created formula template: {}", path.display());
    println!("  Edit it and run 'dl formula check {}' to validate.", path.display());
    
    Ok(())
}

/// Validate a formula file
pub fn check(path: &Path) -> Result<()> {
    let formula = Formula::from_file(path)
        .context("Failed to parse formula")?;
    
    println!("✓ Formula is valid");
    println!("  Name: {}", formula.package.name);
    println!("  Version: {}", formula.package.version);
    
    if let Some(bottle) = formula.bottle_for_current_arch() {
        println!("  Bottle: {} ({})", bottle.url, bottle.arch);
    } else {
        println!("  ⚠ No bottle for current architecture");
    }
    
    Ok(())
}

/// Bump a formula version and update hashes
pub async fn bump(path: &Path, version: &str, url: &str) -> Result<()> {
    println!("🚀 Bumping {} to {}...", path.display(), version);
    
    // Download and compute hash
    println!("📦 Downloading new bottle to compute hash...");
    
    let temp_dir = tempfile::tempdir()?;
    let temp_file = temp_dir.path().join("download");
    
    let client = reqwest::Client::new();
    let response = client.get(url).send().await
        .context("Failed to download")?;
    
    if !response.status().is_success() {
        anyhow::bail!("Download failed: HTTP {}", response.status());
    }
    
    let bytes = response.bytes().await?;
    std::fs::write(&temp_file, &bytes)?;
    
    // Compute hash
    let hash = compute_file_hash(&temp_file)?;
    println!("✓ Computed hash: {}", hash);
    
    // Update formula file - simple approach: read TOML, update, write back
    let mut formula = apl::formula::Formula::from_file(path)?;
    formula.package.version = version.to_string();
    
    // Update the bottle URL and hash for current arch
    let arch = apl::arch::current();
    if let Some(bottle) = formula.bottle.get_mut(arch) {
        bottle.url = url.to_string();
        bottle.blake3 = hash.clone();
    } else {
        formula.bottle.insert(arch.to_string(), apl::formula::Bottle {
            arch: arch.to_string(),
            url: url.to_string(),
            blake3: hash.clone(),
            macos: "11.0".to_string(),
        });
    }
    
    // Serialize back to TOML
    let updated = toml::to_string_pretty(&formula)?;
    
    std::fs::write(path, &updated)?;
    println!("✓ Successfully updated {}", path.display());
    
    Ok(())
}

/// Compute BLAKE3 hash of a file
fn compute_file_hash(path: &Path) -> Result<String> {
    use std::io::Read;
    
    let mut file = std::fs::File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0u8; 65536];
    
    loop {
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }
    
    Ok(hasher.finalize().to_hex().to_string())
}
