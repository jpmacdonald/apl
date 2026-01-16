use crate::db::StateDb;
use crate::ui::Reporter;
use crate::{ops::InstallError, store_path};
use apl_schema::types::{PackageName, Version};

/// Updates symlinks and database state to point to a different installed version.
pub fn switch_version<R: Reporter>(
    name: &PackageName,
    version: &Version,
    dry_run: bool,
    reporter: &R,
) -> Result<(), InstallError> {
    let db = StateDb::open().map_err(|e| InstallError::context("Failed to open database", e))?;
    let pkg = db
        .get_package_version(name.as_str(), version.as_str())
        .map_err(|e| InstallError::context("Failed to query package version in DB", e))?;

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

            let mut bin_list = Vec::new();
            let meta_path = store_dir.join(".apl-meta.json");
            if meta_path.exists() {
                if let Ok(content) = std::fs::read_to_string(&meta_path) {
                    if let Ok(meta) = serde_json::from_str::<serde_json::Value>(&content) {
                        if let Some(bins) = meta.get("bin").and_then(|b| b.as_array()) {
                            bin_list = bins
                                .iter()
                                .filter_map(|b| b.as_str().map(|s| s.to_string()))
                                .collect();
                        }
                    }
                }
            }

            let files_to_record = crate::ops::link_binaries(&bin_list, &store_dir)?;

            // Persistence: record active version AND active files
            db.install_complete_package(
                &p.name,
                &p.version,
                &p.sha256,
                p.size_bytes,
                &[], // No artifacts table update needed (already there)
                &files_to_record,
            )
            .map_err(|e| InstallError::context("Failed to update database records", e))?;

            db.add_history(&p.name, "switch", None, Some(&p.version), true)
                .map_err(|e| InstallError::context("Failed to record history entry", e))?;

            reporter.done(
                &PackageName::new(&p.name),
                &Version::from(p.version.as_str()),
                "switched",
                None,
            );
        }
        None => {
            let versions = db
                .list_package_versions(name.as_str())
                .map_err(|e| InstallError::Io(std::io::Error::other(e)))?;
            if versions.is_empty() {
                return Err(InstallError::Validation(format!(
                    "Package '{name}' is not installed."
                )));
            } else {
                let available = versions
                    .iter()
                    .map(|v| v.version.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(InstallError::Validation(format!(
                    "Version '{version}' of '{name}' is not installed.\nInstalled versions: {available}"
                )));
            }
        }
    }

    Ok(())
}
