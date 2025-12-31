use anyhow::Result;
use apl::ui::Output;

/// Install one or more packages.
pub async fn install(packages: &[String], dry_run: bool, verbose: bool) -> Result<()> {
    let reporter = Output::new();
    apl::ops::install::install_packages(&reporter, packages, dry_run, verbose)
        .await
        .map_err(|e| anyhow::anyhow!(e))
}
