//! Remove command

use anyhow::{Context, Result};
use apl::db::StateDb;
use apl::io::output::CliOutput;
use futures::future::join_all;

/// Remove one or more packages
pub async fn remove(packages: &[String], dry_run: bool) -> Result<()> {
    let db = StateDb::open().context("Failed to open state database")?;

    let output = CliOutput::new();

    let mut task_list = Vec::new();

    // 1. Resolve all packages first
    for pkg_name in packages {
        if let Ok(Some(info)) = db.get_package(pkg_name) {
            task_list.push((pkg_name.clone(), Some(info.version)));
        } else {
            // Not found - still add to table with "unknown" version or "-"
            task_list.push((pkg_name.clone(), None));
        }
    }

    if task_list.is_empty() {
        return Ok(());
    }

    output.prepare_pipeline(&task_list);
    let ticker = output.start_tick();
    let mut remove_count = 0;
    let mut handles = Vec::new();

    for (pkg, version_opt) in task_list {
        let version = version_opt.unwrap_or_else(|| "-".to_string());

        // If version is "-", it means it wasn't found in DB
        if version == "-" {
            output.fail(&pkg, "-", "not installed");
            continue;
        }

        let files = db.get_package_files(&pkg).unwrap_or_default();

        if dry_run {
            output.done(&pkg, &version, "(dry run)");
            continue;
        }

        output.set_installing(&pkg, &version); // Re-use "installing" state for "removing"

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
    ticker.abort();

    for (name, version) in results.into_iter().flatten() {
        let _ = db.remove_package(&name);
        let _ = db.add_history(&name, "remove", Some(&version), None, true);
        output.done(&name, &version, "unlinked from bin");
        remove_count += 1;
    }

    if remove_count > 0 {
        output.summary(remove_count, "removed", 0.0);
    } else {
        // If we attempted removal but count is 0, it means all failed or were not found
        output.error_summary("No packages removed");
    }

    if apl::lockfile::Lockfile::exists_default() {
        let _ = crate::cmd::lock::lock(false, true);
    }

    Ok(())
}
