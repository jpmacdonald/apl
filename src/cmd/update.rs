//! Update command

use anyhow::{Context, Result, bail};
use apl::apl_home;
use apl::index::PackageIndex;
use reqwest::Client;

/// Update package index from CDN
pub async fn update(url: &str, dry_run: bool) -> Result<()> {
    let index_path = apl_home().join("index.bin");
    let output = apl::ui::Output::new();

    if dry_run {
        output.info(&format!("Would download index from: {url}"));
        output.info(&format!("Would save to: {}", index_path.display()));
        return Ok(());
    }

    // 1. Check animation (using standalone standalone)
    output.info("Checking for updates...");

    // Simulate check time if strictly local, but we have real network call
    // Let's give it a minimum time so the user sees "Checking..."
    // In a real optimized CLI we might skip this sleep, but for UX feel it's nice.
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    let client = Client::new();
    let response = match client.get(url).send().await {
        Ok(resp) => resp,
        Err(e) => {
            output.error("Failed to check updates");
            return Err(e.into());
        }
    };

    if !response.status().is_success() {
        output.error(&format!("HTTP {}", response.status()));
        bail!("Failed to fetch index: HTTP {}", response.status());
    }

    let bytes = response.bytes().await?;

    // Auto-detect ZSTD compression
    let decompressed = if bytes.len() >= 4
        && bytes[0] == 0x28
        && bytes[1] == 0xB5
        && bytes[2] == 0x2F
        && bytes[3] == 0xFD
    {
        zstd::decode_all(bytes.as_ref()).context("Failed to decompress index")?
    } else {
        bytes.to_vec()
    };

    let index = PackageIndex::from_bytes(&decompressed).context("Invalid index format")?;

    // Load current index for comparison
    let current_index = PackageIndex::load(&index_path).ok();

    // Stop checking animation (handled by finish_standalone)

    if let Some(current) = current_index {
        if current.updated_at == index.updated_at {
            output.success("Index already up to date");
            return Ok(());
        }
    }

    output.success("Index updated");

    // Save RAW (decompressed) data to disk for fast MMAP loading
    std::fs::write(&index_path, &decompressed)?;

    // 2. Show updates table
    let db = apl::db::StateDb::open()?;
    let packages = db.list_packages()?;
    let mut update_list = Vec::new();

    for pkg in &packages {
        if let Some(entry) = index.find(&pkg.name) {
            let latest = entry.latest().version.clone();
            if latest != pkg.version {
                update_list.push((pkg.name.clone(), pkg.version.clone(), latest));
            }
        }
    }

    // Show available updates (upgrade command actually installs them)
    if !update_list.is_empty() {
        output.info(&format!(
            "{} package(s) can be upgraded:",
            update_list.len()
        ));
        for (name, old, new) in &update_list {
            println!("  {name}: {old} -> {new}");
        }
        output.info(
            "Run 'apl upgrade' to upgrade all, or 'apl upgrade <package>' for specific ones.",
        );
    }

    Ok(())
}
