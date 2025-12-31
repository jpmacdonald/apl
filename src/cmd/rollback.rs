//! Rollback command

use anyhow::{Context, Result, bail};
use apl::db::StateDb;
// use apl::ui::Output; // Not needed if switch handles output

/// Rollback a package to its previous state
pub async fn rollback(pkg_name: &str, dry_run: bool) -> Result<()> {
    let db = StateDb::open().context("Failed to open state database")?;

    // Find last SUCCESSFUL action
    let last_event = db.get_last_successful_history(pkg_name)?;

    let event = match last_event {
        Some(e) => e,
        None => bail!("No history found for '{pkg_name}', cannot rollback."),
    };

    // Determine target version
    let target_version = match event.version_from {
        Some(v) => v,
        None => {
            // If last action was install (with no previous version), rollback means removing
            if event.action == "install" {
                println!("Last action was fresh install of {pkg_name}. Removing...");
                crate::cmd::remove::remove(&[pkg_name.to_string()], false, false, false, dry_run)
                    .await?;
                return Ok(());
            }
            bail!(
                "Cannot rollback: previous state unknown (action: {})",
                event.action
            );
        }
    };

    // Verify target exists
    if db.get_package_version(pkg_name, &target_version)?.is_none() {
        bail!(
            "Target version '{target_version}' is not installed (might have been removed). Cannot rollback."
        );
    }

    let output = apl::ui::Output::new();
    output.info(&format!("Rolling back {pkg_name} to {target_version}..."));

    // Execute switch using the shared logic
    apl::ops::switch::switch_version(pkg_name, &target_version, dry_run, &output)
        .map_err(|e| anyhow::anyhow!(e))?;

    Ok(())
}
