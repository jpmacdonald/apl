//! Lock command

use anyhow::{Context, Result};
use dl::db::StateDb;
use dl::lockfile::{Lockfile, LockedPackage};

/// Generate dl.lock from installed packages
pub fn lock(dry_run: bool) -> Result<()> {
    let db = StateDb::open().context("Failed to open state database")?;
    let packages = db.list_packages()?;
    
    if packages.is_empty() {
        println!("No packages installed. Nothing to lock.");
        return Ok(());
    }
    
    let lock_path = std::env::current_dir()?.join("dl.lock");
    
    if dry_run {
        println!("Would generate dl.lock with {} packages:", packages.len());
        for pkg in &packages {
            println!("  {} {}", pkg.name, pkg.version);
        }
        return Ok(());
    }
    
    // Build lockfile from installed packages
    let locked_packages: Vec<LockedPackage> = packages.iter()
        .map(|pkg| LockedPackage {
            name: pkg.name.clone(),
            version: pkg.version.clone(),
            blake3: pkg.blake3.clone(),
            url: String::new(), // We don't have the URL stored in DB; could look up in index
            arch: dl::arch::current().to_string(),
        })
        .collect();
    
    let lockfile = Lockfile {
        version: 1,
        generated_at: chrono_lite_now(),
        packages: locked_packages,
    };
    
    lockfile.save(&lock_path)?;
    
    println!("✓ Generated dl.lock with {} packages", packages.len());
    
    Ok(())
}

/// Get current timestamp as string
fn chrono_lite_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    format!("{}", secs)
}
