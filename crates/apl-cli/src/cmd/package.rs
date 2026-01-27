//! Package management commands

use anyhow::{Context, Result};
use apl_core::package::Package;
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
sha256 = "PLACEHOLDER"
format = "tar.gz"

[binary.arm64]
url = "https://example.com/releases/download/v0.1.0/{name}-0.1.0-arm64.tar.gz"
sha256 = "PLACEHOLDER"
format = "tar.gz"

[binary.x86_64]
url = "https://example.com/releases/download/v0.1.0/{name}-0.1.0-x86_64.tar.gz"
sha256 = "PLACEHOLDER"
format = "tar.gz"

[install]
strategy = "link"
bin = ["{name}"]

[dependencies]
"#
    );

    let output = crate::ui::Output::new();
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
    let content = std::fs::read_to_string(path).context("Failed to read package file")?;
    let output = crate::ui::Output::new();

    if let Ok(template) = apl_core::package::PackageTemplate::parse(&content) {
        output.success("Package (Template) is valid");
        println!("  Name: {}", template.package.name);
        println!(
            "  Source: {}",
            template.discovery.github_repo().unwrap_or_default()
        );
        return Ok(());
    }

    let pkg = Package::parse(&content).context("Failed to parse package")?;
    output.success("Package (Standard) is valid");
    println!("  Name: {}", pkg.package.name);
    println!("  Version: {}", pkg.package.version);

    if pkg.source.url.is_empty() {
        output.warning("No source URL defined");
    } else {
        println!("  Source: {}", pkg.source.url);
    }

    Ok(())
}

/// Bump a package version (mostly legacy, but keeping skeleton for now)
pub fn bump(path: &Path, version: &str, _url: &str) -> Result<()> {
    let output = crate::ui::Output::new();
    output.info(&format!("Bumping {} to {}...", path.display(), version));

    // Handle updates via template or direct manipulation of State 2 packages
    // For now, this is disabled until we decide on the new user-side package format.
    output.warning("Bump command is being refactored for the Selectors Pattern.");
    Ok(())
}
