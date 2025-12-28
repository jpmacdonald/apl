//! Switch command to change active package version

use anyhow::{Context, Result, bail};
use apl::core::version::PackageSpec;
use apl::db::StateDb;
use apl::cas::Cas;
use crate::cmd::install::finalize_switch;

/// Switch the active version of a package
pub fn switch(pkg_spec: &str, dry_run: bool) -> Result<()> {
    // Parse input (can be "name@version" or just "name" if we supported interactive, but for now strict)
    let spec = PackageSpec::parse(pkg_spec)?;
    let version = spec.version.context("Version is required for switch (e.g., 'dl switch jq@1.6')")?;
    
    let db = StateDb::open().context("Failed to open state database")?;
    
    // Check if valid package
    // Note: get_package_version returns information if that SPECIFIC version is installed
    let pkg = db.get_package_version(&spec.name, &version)?;
    
    match pkg {
        Some(p) => {
            if p.active {
                println!("✓ {} {} is already active", p.name, p.version);
                return Ok(());
            }
            
            // Proceed to switch
            let cas = Cas::new()?;
            finalize_switch(&cas, &db, &p.name, &p.version, dry_run)?;
        }
        None => {
            // Check if package is installed at all (maybe user made typo in version)
            let versions = db.list_package_versions(&spec.name)?;
            if versions.is_empty() {
                bail!("Package '{}' is not installed.", spec.name);
            } else {
                let available = versions.iter().map(|v| v.version.as_str()).collect::<Vec<_>>().join(", ");
                bail!("Version '{}' of '{}' is not installed.\nInstalled versions: {}", 
                    version, spec.name, available);
            }
        }
    }
    
    Ok(())
}
