use anyhow::{Context, Result};
use apl::ui::Output;
use apl::{DbHandle, apl_home};
use std::sync::Arc;

/// Install one or more packages.
pub async fn install(packages: &[String], dry_run: bool, _verbose: bool) -> Result<()> {
    let reporter = Arc::new(Output::new());

    // Initialize dependencies formerly done inside install_packages
    let db = DbHandle::spawn().context("Failed to open database")?;
    let index_path = apl_home().join("index");
    let index = apl::core::index::PackageIndex::load(&index_path).ok();

    let client = reqwest::Client::builder()
        .tcp_nodelay(true)
        .pool_max_idle_per_host(20)
        .build()
        .context("Failed to build HTTP client")?;

    let ctx = apl::ops::Context::new(db, index, client, reporter);

    apl::ops::install::install_packages(&ctx, packages, dry_run)
        .await
        .map_err(|e| anyhow::anyhow!(e))
}
