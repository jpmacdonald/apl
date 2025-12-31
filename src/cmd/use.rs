//! Switch command to change active package version (aliased as 'use')
use anyhow::{Context, Result};
use apl::core::version::PackageSpec;

/// Switch the active version of a package (CLI Entry Point)
pub fn use_package(pkg_spec: &str, dry_run: bool) -> Result<()> {
    // Parse input
    let spec = PackageSpec::parse(pkg_spec)?;
    let version = spec
        .version()
        .map(|v| v.to_string())
        .context("Version is required for use (e.g., 'apl use jq@1.6')")?;

    apl::ops::switch::switch_version(&spec.name, &version, dry_run)
}

/// Perform the switch to a specific version (Reusable logic)
/// Deprecated: Use ops::switch::switch_version directly
pub fn use_version(name: &str, version: &str, dry_run: bool) -> Result<()> {
    apl::ops::switch::switch_version(name, version, dry_run)
}
