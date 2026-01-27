//! Rollback command

use anyhow::{Context, Result, bail};
use crate::db::StateDb;
// use crate::ui::Output; // Not needed if switch handles output

/// Rollback a package to its previous state
pub async fn rollback(pkg_name: &str, dry_run: bool) -> Result<()> {
    let db = StateDb::open().context("Failed to open state database")?;

    // Find last SUCCESSFUL action
    let last_event = db.get_last_successful_history(pkg_name)?;

    let Some(event) = last_event else {
        bail!("No history found for '{pkg_name}', cannot rollback.");
    };

    // Determine target version
    let Some(target_version) = event.version_from else {
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
    };

    // Verify target exists
    if db.get_package_version(pkg_name, &target_version)?.is_none() {
        bail!(
            "Target version '{target_version}' is not installed (might have been removed). Cannot rollback."
        );
    }

    let output = crate::ui::Output::new();
    let name = apl_schema::types::PackageName::new(pkg_name);
    let version = apl_schema::types::Version::from(target_version);
    output.info(&format!("Rolling back {pkg_name} to {version}..."));

    // Execute switch using the shared logic
    crate::ops::switch::switch_version(&name, &version, dry_run, &output)
        .map_err(|e| anyhow::anyhow!(e))?;

    Ok(())
}
