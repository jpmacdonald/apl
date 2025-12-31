//! Switch command to change active package version (aliased as 'use')
use anyhow::{Context, Result};
use apl::core::version::PackageSpec;

/// Switch the active version of a package (CLI Entry Point)
pub fn use_package(pkg_spec: &str, dry_run: bool) -> Result<()> {
    let output = apl::ui::Output::new();

    // Parse input
    let spec = PackageSpec::parse(pkg_spec)?;
    let version = spec
        .version()
        .map(|v| v.to_string())
        .context("Version is required for use (e.g., 'apl use jq@1.6')")?;

    apl::ops::switch::switch_version(&spec.name, &version, dry_run, &output)
        .map_err(|e| anyhow::anyhow!(e))
}
