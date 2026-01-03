use futures::future::join_all;
use std::path::PathBuf;
use std::time::Instant;

use crate::DbHandle;
use crate::ops::InstallError;
use crate::types::{PackageName, Version};
use crate::ui::Reporter;

/// Resolves and removes packages, deleting active files and updating the database.
pub async fn remove_packages<R: Reporter + Clone + 'static>(
    reporter: &R,
    packages: &[String],
    force: bool,
    dry_run: bool,
) -> Result<(), InstallError> {
    let db = DbHandle::spawn().map_err(|e| InstallError::Io(std::io::Error::other(e)))?;

    let mut task_list: Vec<(PackageName, Option<Version>)> = Vec::new();

    // 1. Resolve package status
    for pkg_name_str in packages {
        let pkg_name = PackageName::new(pkg_name_str);
        if let Ok(Some(info)) = db.get_package(pkg_name.to_string()).await {
            task_list.push((pkg_name, Some(Version::from(info.version))));
        } else {
            task_list.push((pkg_name, None));
        }
    }

    if task_list.is_empty() {
        return Ok(());
    }

    let start_time = Instant::now();
    reporter.prepare_pipeline(&task_list);

    let mut remove_count = 0;
    let mut handles = Vec::new();

    for (pkg, version_opt) in task_list {
        if version_opt.is_none() {
            reporter.failed(&pkg, &Version::from("-"), "not installed");
            continue;
        }
        let version = version_opt.unwrap();

        // Get files for this package
        let files = db
            .get_package_files(pkg.to_string())
            .await
            .map_err(|e| InstallError::Io(std::io::Error::other(e)))?;

        if files.is_empty() && !force {
            reporter.failed(&pkg, &version, "no files tracked");
            continue;
        }
        if files.is_empty() && force {
            reporter.warning(&format!(
                "No files tracked for {pkg}, but forcing metadata cleanup"
            ));
        }

        reporter.removing(&pkg, &version);

        let files_to_delete = files;
        let pkg_name = pkg;
        let version_final = version;
        let reporter_clone = reporter.clone();

        handles.push(tokio::spawn(async move {
            let mut success = true;
            if !dry_run {
                for file_record in files_to_delete {
                    let path = PathBuf::from(&file_record.path);
                    if path.exists() {
                        let is_app_bundle = file_record.sha256 == "APP_BUNDLE";
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
                Some((pkg_name, version_final))
            } else {
                reporter_clone.failed(&pkg_name, &version_final, "partial removal");
                None
            }
        }));
    }

    let results = join_all(handles).await;

    for (name, version) in results.into_iter().flatten().flatten() {
        if !dry_run {
            let _ = db.remove_package(name.to_string()).await;
            let _ = db
                .add_history(
                    name.to_string(),
                    "remove".to_string(),
                    Some(version.to_string()),
                    None,
                    true,
                )
                .await;
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

    if remove_count > 0 {
        reporter.summary(remove_count, "removed", start_time.elapsed().as_secs_f64());
    }

    Ok(())
}
