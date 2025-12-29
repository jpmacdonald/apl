//! Remove command

use anyhow::{Context, Result};
use futures::future::join_all;
use apl::db::StateDb;
use apl::io::output::InstallOutput;

/// Remove one or more packages
pub async fn remove(packages: &[String], dry_run: bool) -> Result<()> {
    let db = StateDb::open().context("Failed to open state database")?;
    
    let output = InstallOutput::new(false);
    output.section("Removing");

    let mut remove_count = 0;
    let mut handles = Vec::new();

    for pkg in packages {
        let pkg_info = db.get_package(pkg).ok().flatten();
        
        if pkg_info.is_none() {
             output.warn(&format!("Package '{}' is not installed", pkg));
             continue;
        }
        
        let files = db.get_package_files(pkg).unwrap_or_default();
        let version = pkg_info.map(|p| p.version).unwrap_or_else(|| "unknown".to_string());

        if dry_run {
            output.done(pkg, &version, "(dry run)");
            continue;
        }
        
        let files_to_delete: Vec<String> = files.iter().map(|f| f.path.clone()).collect();
        let pkg_name = pkg.clone();
        let version_clone = version.clone();

        handles.push(tokio::task::spawn_blocking(move || {
            for file_path in files_to_delete {
                let path = std::path::Path::new(&file_path);
                let _ = if path.is_dir() {
                    std::fs::remove_dir_all(path)
                } else {
                    std::fs::remove_file(path)
                };
            }
            (pkg_name, version_clone)
        }));
    }

    let results = join_all(handles).await;

    for res in results {
        if let Ok((name, version)) = res {
             // Record in DB and History
             let _ = db.remove_package(&name);
             let _ = db.add_history(&name, "remove", Some(&version), None, true);
             output.done(&name, &version, "done");
             remove_count += 1;
        }
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
        let _ = crate::cmd::lock::lock(false, true);
    }

    Ok(())
}
