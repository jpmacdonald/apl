//! Remove command

use anyhow::{Context, Result};
use apl::db::StateDb;
use apl::ui::Output;
use crossterm::style::Stylize;
use futures::future::join_all;
use std::path::PathBuf;
use std::time::Instant;

/// Remove one or more packages
pub async fn remove(packages: &[String], all: bool, yes: bool, dry_run: bool) -> Result<()> {
    let db = StateDb::open().context("Failed to open state database")?;

    let packages_to_remove = if all {
        let all_packages = db.list_packages()?;
        if all_packages.is_empty() {
            println!("  ℹ No packages installed.");
            return Ok(());
        }

        if !yes {
            println!();
            print!(
                "  ⚠ {} This will remove all installed packages. Continue? (y/N) ",
                "WARNING:".bold().red()
            );
            use std::io::Write;
            std::io::stdout().flush()?;

            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            if !input.trim().eq_ignore_ascii_case("y") {
                let output = Output::new();
                output.error("Operation cancelled");
                return Ok(());
            }
        }

        all_packages.into_iter().map(|p| p.name).collect()
    } else {
        packages.to_vec()
    };

    if packages_to_remove.is_empty() {
        return Ok(());
    }

    let output = Output::new();
    let mut task_list = Vec::new();

    // 1. Resolve all packages first
    for pkg_name in &packages_to_remove {
        if let Ok(Some(info)) = db.get_package(pkg_name) {
            task_list.push((pkg_name.clone(), Some(info.version)));
        } else {
            task_list.push((pkg_name.clone(), None));
        }
    }

    if task_list.is_empty() {
        return Ok(());
    }

    let start_time = Instant::now();
    output.prepare_pipeline(&task_list);

    let mut remove_count = 0;
    let mut handles = Vec::new();

    for (pkg, version_opt) in task_list {
        if version_opt.is_none() {
            output.failed(&pkg, "-", "not installed");
            continue;
        }
        let version = version_opt.unwrap();

        // Get files for this package
        let files = db.get_package_files(&pkg)?;
        if files.is_empty() {
            output.failed(&pkg, &version, "no files tracked");
            continue;
        }

        output.removing(&pkg, &version);

        let files_to_delete = files;
        let pkg_name = pkg.clone();
        let version_str = version.clone();
        let output_clone = output.clone();

        handles.push(tokio::spawn(async move {
            let mut success = true;
            if !dry_run {
                for file_record in files_to_delete {
                    let path = PathBuf::from(&file_record.path);
                    if path.exists() {
                        let is_app_bundle = file_record.blake3 == "APP_BUNDLE";
                        let is_dir = path.is_dir();

                        let result = if is_app_bundle || is_dir {
                            std::fs::remove_dir_all(&path)
                        } else {
                            std::fs::remove_file(&path)
                        };

                        if result.is_err() {
                            success = false;
                        }
                    }
                }
            }
            if success {
                Some((pkg_name, version_str))
            } else {
                output_clone.failed(&pkg_name, &version_str, "partial removal");
                None
            }
        }));
    }

    let results = join_all(handles).await;

    for res in results {
        if let Ok(Some((name, version))) = res {
            if !dry_run {
                let _ = db.remove_package(&name);
                let _ = db.add_history(&name, "remove", Some(&version), None, true);
            }
            output.done(
                &name,
                &version,
                if dry_run {
                    "(dry run)"
                } else {
                    "unlinked from bin"
                },
                None,
            );
            remove_count += 1;
        }
    }

    if remove_count > 0 {
        output.summary(remove_count, "removed", start_time.elapsed().as_secs_f64());
    } else {
        output.error_summary("No packages removed");
    }

    // Sync UI actor
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    Ok(())
}
