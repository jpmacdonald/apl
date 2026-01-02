//! Package management commands

use anyhow::{Context, Result};
use apl::Version;
use apl::package::Package;
use std::path::Path;

/// Create a new package template
pub fn new(name: &str, output_dir: &Path) -> Result<()> {
    let filename = format!("{name}.toml");
    let path = output_dir.join(&filename);

    if path.exists() {
        anyhow::bail!("Package already exists: {}", path.display());
    }

    let template = format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
description = ""
homepage = ""
type = "cli"

[source]
url = "https://github.com/OWNER/{name}/archive/refs/tags/v0.1.0.tar.gz"
blake3 = "PLACEHOLDER"
format = "tar.gz"

[binary.arm64]
url = "https://example.com/releases/download/v0.1.0/{name}-0.1.0-arm64.tar.gz"
blake3 = "PLACEHOLDER"
format = "tar.gz"

[binary.x86_64]
url = "https://example.com/releases/download/v0.1.0/{name}-0.1.0-x86_64.tar.gz"
blake3 = "PLACEHOLDER"
format = "tar.gz"

[install]
strategy = "link"
bin = ["{name}"]

[dependencies]
"#
    );

    let output = apl::ui::Output::new();
    std::fs::create_dir_all(output_dir)?;
    std::fs::write(&path, template)?;

    output.success(&format!("Created package template: {}", path.display()));
    output.info(&format!(
        "Edit it and run 'apl package check {}' to validate.",
        path.display()
    ));

    Ok(())
}

/// Validate a package file
pub fn check(path: &Path) -> Result<()> {
    let pkg = Package::from_file(path).context("Failed to parse package")?;

    let output = apl::ui::Output::new();
    output.success("Package is valid");
    println!("  Name: {}", pkg.package.name);
    println!("  Version: {}", pkg.package.version);

    if let Some(binary) = pkg.binary_for_current_arch() {
        println!("  Binary: {} ({})", binary.url, binary.arch);
    } else {
        output.warning("No binary for current architecture");
    }

    Ok(())
}

/// Bump a package version and update hashes
pub async fn bump(path: &Path, version: &str, url: &str) -> Result<()> {
    let output = apl::ui::Output::new();
    output.info(&format!("Bumping {} to {}...", path.display(), version));

    // Download and compute hash
    output.info("Downloading new binary to compute hash...");

    let temp_dir = tempfile::tempdir()?;
    let temp_file = temp_dir.path().join("download");

    let client = reqwest::Client::new();
    let response = client.get(url).send().await.context("Failed to download")?;

    if !response.status().is_success() {
        anyhow::bail!("Download failed: HTTP {}", response.status());
    }

    let bytes = response.bytes().await?;
    std::fs::write(&temp_file, &bytes)?;

    // Compute hash
    let hash = compute_file_hash(&temp_file)?;
    output.success(&format!("Computed hash: {hash}"));

    // Update package file
    let mut pkg = apl::package::Package::from_file(path)?;
    pkg.package.version = Version::from(version.to_string());

    // Update the binary URL and hash for current arch
    let arch = apl::Arch::current();
    if let Some(binary) = pkg.binary.get_mut(&arch) {
        binary.url = url.to_string();
        binary.blake3 = hash.clone();
    } else {
        pkg.binary.insert(
            arch,
            apl::package::Binary {
                arch,
                url: url.to_string(),
                blake3: hash.clone(),
                format: apl::package::ArtifactFormat::Binary,
                macos: "11.0".to_string(),
            },
        );
    }

    // Serialize back to TOML
    let updated = toml::to_string_pretty(&pkg)?;

    std::fs::write(path, &updated)?;
    output.success(&format!("Successfully updated {}", path.display()));

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
