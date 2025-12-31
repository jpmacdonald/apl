use futures::future::join_all;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use crate::db::StateDb;
use crate::ops::InstallError;
use crate::ui::Reporter;

/// Resolves and removes packages, deleting active files and updating the database.
pub async fn remove_packages<R: Reporter + Clone + 'static>(
    reporter: &R,
    packages: &[String],
    force: bool,
    dry_run: bool,
) -> Result<(), InstallError> {
    let db = StateDb::open().map_err(|e| InstallError::Io(std::io::Error::other(e)))?;

    let mut task_list = Vec::new();

    // 1. Resolve package status
    for pkg_name in packages {
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
    reporter.prepare_pipeline(&task_list);

    let mut remove_count = 0;
    let mut handles = Vec::new();
    let db_arc = Arc::new(db);

    for (pkg, version_opt) in task_list {
        if version_opt.is_none() {
            reporter.failed(&pkg, "-", "not installed");
            continue;
        }
        let version = version_opt.unwrap();

        // Get files for this package
        let files = db_arc
            .get_package_files(&pkg)
            .map_err(|e| InstallError::Io(std::io::Error::other(e)))?;

        if files.is_empty() && !force {
            reporter.failed(&pkg, &version, "no files tracked");
            continue;
        }

        if files.is_empty() && force {
            reporter.warning(&format!(
                "No files tracked for {}, but forcing metadata cleanup",
                pkg
            ));
        }

        reporter.removing(&pkg, &version);

        let files_to_delete = files;
        let pkg_name = pkg.clone();
        let version_str = version.clone();
        let reporter_clone = reporter.clone();

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
                reporter_clone.failed(&pkg_name, &version_str, "partial removal");
                None
            }
        }));
    }

    let results = join_all(handles).await;

    for res in results {
        if let Ok(Some((name, version))) = res {
            if !dry_run {
                let _ = db_arc.remove_package(&name);
                let _ = db_arc.add_history(&name, "remove", Some(&version), None, true);
            }
            reporter.done(
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
        reporter.summary(remove_count, "removed", start_time.elapsed().as_secs_f64());
    }

    Ok(())
}
