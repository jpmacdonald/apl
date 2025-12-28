//! Rollback command

use anyhow::{Context, Result, bail};
use apl::db::StateDb;
use apl::cas::Cas;
use apl::io::output::InstallOutput;
use crate::cmd::install::finalize_switch;

/// Rollback a package to its previous state
pub fn rollback(pkg_name: &str, dry_run: bool) -> Result<()> {
    let db = StateDb::open().context("Failed to open state database")?;
    
    // Find last SUCCESSFUL action
    let last_event = db.get_last_successful_history(pkg_name)?;
    
    let event = match last_event {
        Some(e) => e,
        None => bail!("No history found for '{}', cannot rollback.", pkg_name),
    };
    
    // Determine target version
    let target_version = match event.version_from {
        Some(v) => v,
        None => {
             // If last action was install (with no previous version), rollback means removing
             if event.action == "install" {
                 println!("Last action was fresh install of {}. Removing...", pkg_name);
                 crate::cmd::remove::remove(&[pkg_name.to_string()], dry_run)?;
                 return Ok(());
             }
             bail!("Cannot rollback: previous state unknown (action: {})", event.action);
        }
    };

    // Verify target exists
    if db.get_package_version(pkg_name, &target_version)?.is_none() {
        bail!("Target version '{}' is not installed (might have been removed). Cannot rollback.", target_version);
    }
    
    println!("Rolling back {} to {}...", pkg_name, target_version);
    
    // Execute switch
    let cas = Cas::new()?;
    let output = InstallOutput::new(false);
    finalize_switch(&cas, &db, pkg_name, &target_version, dry_run, &output)?;
    
    Ok(())
}
