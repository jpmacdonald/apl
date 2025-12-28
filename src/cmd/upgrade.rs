//! Upgrade command

use anyhow::{Context, Result, bail};
use apl::db::StateDb;
use apl::index::PackageIndex;
use apl::apl_home;

/// Upgrade installed packages to latest versions
pub async fn upgrade(packages: &[String], dry_run: bool) -> Result<()> {
    let db = StateDb::open().context("Failed to open state database")?;
    
    if dry_run {
        println!("Checking for upgrades...");
    }
    
    let index_path = apl_home().join("index.bin");
    if !index_path.exists() {
        bail!("No index found. Run 'dl update' first.");
    }
    
    let index = PackageIndex::load(&index_path)
        .context("Failed to load index")?;
    
    // Get list of packages to check
    let installed = db.list_packages()?;
    let to_check: Vec<_> = if packages.is_empty() {
        installed.iter().map(|p| p.name.clone()).collect()
    } else {
        packages.to_vec()
    };
    
    let mut upgrades = Vec::new();
    
    for pkg_name in &to_check {
        // Get installed version
        let installed_pkg = db.get_package(pkg_name)?;
        let Some(installed_pkg) = installed_pkg else {
            if !packages.is_empty() {
                println!("  {} is not installed", pkg_name);
            }
            continue;
        };
        
        // Get latest version from index
        let Some(index_entry) = index.find(pkg_name) else {
            continue; // Package not in index
        };
        
        if installed_pkg.version != index_entry.latest().version {
            upgrades.push((pkg_name.clone(), installed_pkg.version.clone(), index_entry.latest().version.clone()));
        }
    }
    
    if upgrades.is_empty() {
        println!("✓ All packages are up to date");
        return Ok(());
    }
    
    println!("📦 Upgrades available:");
    for (name, old, new) in &upgrades {
        println!("  {} {} → {}", name, old, new);
    }
    
    if dry_run {
        return Ok(());
    }
    
    // Perform upgrades
    let upgrade_names: Vec<String> = upgrades.into_iter().map(|(n, _, _)| n.to_string()).collect();
    crate::cmd::install::install(&upgrade_names, false, false).await?;
    
    Ok(())
}
