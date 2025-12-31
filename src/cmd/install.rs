//! Install command
use anyhow::Result;

/// Install one or more packages
pub async fn install(packages: &[String], dry_run: bool, verbose: bool) -> Result<()> {
    apl::ops::install::install_packages(packages, dry_run, verbose).await
}
