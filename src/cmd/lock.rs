//! Lock command

use anyhow::{Context, Result};
use apl::db::StateDb;
use apl::lockfile::{LockedPackage, Lockfile};

/// Generate apl.lock from installed packages
pub fn lock(dry_run: bool, silent: bool) -> Result<()> {
    let db = StateDb::open().context("Failed to open state database")?;
    let packages = db.list_packages()?;

    let output = apl::io::output::CliOutput::new();

    if packages.is_empty() && !silent {
        output.info("No packages installed. generating empty lockfile.");
    }

    let lock_path = std::env::current_dir()?.join("apl.lock");

    if dry_run {
        output.info(&format!(
            "Would generate apl.lock with {} packages:",
            packages.len()
        ));
        for pkg in &packages {
            println!("  {} {}", pkg.name, pkg.version);
        }
        return Ok(());
    }

    // Load index to look up URLs
    let index_path = apl::apl_home().join("index.bin");
    let index = if index_path.exists() {
        Some(apl::index::PackageIndex::load(&index_path).context("Failed to load index")?)
    } else {
        None
    };

    if index.is_none() && !silent {
        output.warning("No index found. Lockfile will contain empty URLs.");
    }

    // Build lockfile from installed packages
    let mut locked_packages = Vec::new();
    let current_arch = apl::arch::current();

    for pkg in packages {
        let mut url = String::new();
        let blake3 = pkg.blake3.clone();

        // Try to find URL in index
        if let Some(idx) = &index {
            if let Some(entry) = idx.find(&pkg.name) {
                if let Some(release) = entry.find_version(&pkg.version) {
                    if let Some(bottle) = release.bottles.iter().find(|b| b.arch == current_arch) {
                        url = bottle.url.clone();
                        // Verify hash matches what we have installed (sanity check)
                        if bottle.blake3 != blake3 && !silent {
                            output.warning(&format!(
                                "Installed hash for {} disagrees with index",
                                pkg.name
                            ));
                        }
                    }
                }
            }
        }

        locked_packages.push(LockedPackage {
            name: pkg.name.clone(),
            version: pkg.version.clone(),
            blake3,
            url,
            arch: current_arch.to_string(),
        });
    }

    let lockfile = Lockfile {
        version: 1,
        generated_at: chrono_lite_now(),
        packages: locked_packages,
    };

    lockfile.save(&lock_path)?;

    if !silent {
        output.success(&format!(
            "Generated apl.lock with {} packages",
            lockfile.packages.len()
        ));
    }

    Ok(())
}

/// Get current timestamp as string
fn chrono_lite_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    format!("{secs}")
}
