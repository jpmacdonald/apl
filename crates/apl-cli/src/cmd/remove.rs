//! Remove command
use crate::db::StateDb;
use crate::ui::Output;
use anyhow::{Context, Result};
use crossterm::style::Stylize;

/// Remove one or more packages
#[allow(clippy::fn_params_excessive_bools)]
pub async fn remove(
    packages: &[String],
    all: bool,
    yes: bool,
    force: bool,
    dry_run: bool,
) -> Result<()> {
    let db = StateDb::open().context("Failed to open state database")?;
    let reporter = Output::new();

    let packages_to_remove = if all {
        let all_packages = db.list_packages()?;
        if all_packages.is_empty() {
            reporter.info("No packages installed.");
            return Ok(());
        }

        if !yes {
            use std::io::Write;
            println!();
            print!(
                "  {} This will remove all installed packages. Continue? (y/N) ",
                "WARNING:".bold().red()
            );
            std::io::stdout().flush()?;

            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            if !input.trim().eq_ignore_ascii_case("y") {
                reporter.error("Operation cancelled");
                return Ok(());
            }
        }

        all_packages.into_iter().map(|p| p.name).collect()
    } else {
        packages.to_vec()
    };

    if packages_to_remove.is_empty() {
        return Ok(());
    }

    crate::ops::remove::remove_packages(&reporter, &packages_to_remove, force, dry_run)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    // Brief sleep to ensure UI actor completes rendering
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    Ok(())
}
