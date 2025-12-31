use crate::db::StateDb;
use crate::ui::Reporter;
use crate::{bin_path, ops::InstallError, store_path};

/// Updates symlinks and database state to point to a different installed version.
pub fn switch_version<R: Reporter>(
    name: &str,
    version: &str,
    dry_run: bool,
    reporter: &R,
) -> Result<(), InstallError> {
    let db = StateDb::open().map_err(|e| InstallError::Io(std::io::Error::other(e)))?;
    let pkg = db
        .get_package_version(name, version)
        .map_err(|e| InstallError::Io(std::io::Error::other(e)))?;

    match pkg {
        Some(p) => {
            if p.active {
                // If the requested version is already active, we have nothing to do.
                return Ok(());
            }

            if dry_run {
                reporter.info(&format!(
                    "(dry run) Would switch {} to {}",
                    p.name, p.version
                ));
                return Ok(());
            }

            let store_dir = store_path().join(&p.name).join(&p.version);
            if !store_dir.exists() {
                return Err(InstallError::Validation(format!(
                    "Package artifacts missing at {}",
                    store_dir.display()
                )));
            }

            let mut bins_to_link = Vec::new();
            let bin_dir = store_dir.join("bin");
            let search_dir = if bin_dir.exists() {
                &bin_dir
            } else {
                &store_dir
            };

            if let Ok(entries) = std::fs::read_dir(search_dir) {
                for entry in entries.flatten() {
                    if let Ok(meta) = entry.metadata() {
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;
                            if meta.is_file() && (meta.permissions().mode() & 0o111 != 0) {
                                let name = entry.file_name().to_string_lossy().to_string();
                                bins_to_link.push((name.clone(), name));
                            }
                        }
                    }
                }
            }

            if bins_to_link.is_empty() {
                reporter.warning(&format!(
                    "No binaries found to link for {} {}",
                    p.name, p.version
                ));
            }

            for (src_rel, target_name) in bins_to_link {
                let src_path = search_dir.join(&src_rel);
                let target = bin_path().join(&target_name);

                if target.exists() || target.is_symlink() {
                    let _ = std::fs::remove_file(&target);
                }

                #[cfg(unix)]
                std::os::unix::fs::symlink(&src_path, &target).map_err(InstallError::Io)?;
            }

            // Persistence
            db.install_package(&p.name, &p.version, &p.blake3)
                .map_err(|e| InstallError::Other(e.to_string()))?;

            db.add_history(&p.name, "switch", None, Some(&p.version), true)
                .map_err(|e| InstallError::Other(e.to_string()))?;

            reporter.done(&p.name, &p.version, "switched", None);
        }
        None => {
            let versions = db
                .list_package_versions(name)
                .map_err(|e| InstallError::Io(std::io::Error::other(e)))?;
            if versions.is_empty() {
                return Err(InstallError::Validation(format!(
                    "Package '{}' is not installed.",
                    name
                )));
            } else {
                let available = versions
                    .iter()
                    .map(|v| v.version.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(InstallError::Validation(format!(
                    "Version '{}' of '{}' is not installed.\nInstalled versions: {}",
                    version, name, available
                )));
            }
        }
    }

    Ok(())
}
