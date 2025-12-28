//! Remove command

use anyhow::{Context, Result};
use apl::db::StateDb;
use apl::io::output::{InstallOutput, PackageState};

/// Remove one or more packages
pub fn remove(packages: &[String], dry_run: bool) -> Result<()> {
    let db = StateDb::open().context("Failed to open state database")?;
    
    let output = InstallOutput::new(false);
    output.section("Removing");

    let mut remove_count = 0;
    
    for pkg in packages {
        // Get file list before removing
        let files = db.get_package_files(pkg)?;
        
        if files.is_empty() {
             output.warn(&format!("Package '{}' is not installed", pkg));
             continue;
        }
        
        // Determine version and type for better status
        let pkg_info = db.get_package(pkg).ok().flatten();
        let version = pkg_info.map(|p| p.version).unwrap_or_else(|| "unknown".to_string());

        if dry_run {
            output.package_line(PackageState::Queued, pkg, &version, "(dry run: remove)");
            continue;
        }
        
        output.package_line(PackageState::Installing, pkg, &version, "removing...");

        // Delete files
        for file in &files {
            let path = std::path::Path::new(&file.path);
            let result = if path.is_dir() {
                std::fs::remove_dir_all(path)
            } else {
                std::fs::remove_file(path)
            };
            
            if let Err(e) = result {
                // Only warn in verbose, don't clutter the main feed
                output.verbose(&format!("could not remove {}: {}", file.path, e));
            }
        }
        
        // Remove from DB
        db.remove_package(pkg)?;
        
        // Record history
        db.add_history(pkg, "remove", Some(&version), None, true)?;

        output.package_line(PackageState::Installed, pkg, &version, "done");
        remove_count += 1;
    }

    if remove_count > 0 {
        println!();
        println!("  {} {} package{} removed",
            console::style("✨").green(),
            remove_count,
            if remove_count == 1 { "" } else { "s" },
        );
    }

    // Auto-update lockfile if it exists
    if apl::lockfile::Lockfile::exists_default() {
        let _ = crate::cmd::lock::lock(false, true); // silent
    }

    Ok(())
}
