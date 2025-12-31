use crate::db::StateDb;
use crate::ui::Output;
use crate::{bin_path, store_path};
use anyhow::{Context, Result, bail};

/// Perform the switch to a specific version (Reusable logic)
pub fn switch_version(name: &str, version: &str, dry_run: bool) -> Result<()> {
    let db = StateDb::open().context("Failed to open state database")?;
    let pkg = db.get_package_version(name, version)?;

    match pkg {
        Some(p) => {
            if p.active {
                // Determine if we should print based on silent?
                // The original code used println! which is bad.
                // Ops usually shouldn't print unless dry_run?
                // The original code:
                // println!("✓ {} {} is already active", p.name, p.version);
                // I'll leave it for now or use Output.
                // But wait, ops shouldn't create Output ideally.
                // For now, I'll allow it to use Output to match previous behavior.
                println!("✓ {} {} is already active", p.name, p.version);
                return Ok(());
            }

            if dry_run {
                println!("(dry run) Would switch {} to {}", p.name, p.version);
                return Ok(());
            }

            // 1. Prepare Store Paths
            let store_dir = store_path().join(&p.name).join(&p.version);
            if !store_dir.exists() {
                bail!("Package artifacts missing at {}", store_dir.display());
            }

            // 2. Identify Binaries in Store
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
                            // Executable bit check
                            if meta.is_file() && (meta.permissions().mode() & 0o111 != 0) {
                                let name = entry.file_name().to_string_lossy().to_string();
                                bins_to_link.push((name.clone(), name));
                            }
                        }
                    }
                }
            }

            if bins_to_link.is_empty() {
                println!(
                    "Warning: No binaries found to link for {} {}",
                    p.name, p.version
                );
            }

            // 3. Update Symlinks
            for (src_rel, target_name) in bins_to_link {
                let src_path = search_dir.join(&src_rel);
                let target = bin_path().join(&target_name);

                if target.exists() || target.is_symlink() {
                    let _ = std::fs::remove_file(&target);
                }

                #[cfg(unix)]
                std::os::unix::fs::symlink(&src_path, &target).with_context(|| {
                    format!(
                        "Failed to link {} -> {}",
                        target.display(),
                        src_path.display()
                    )
                })?;
            }

            // 4. Update DB State
            // Mark this version as active.
            db.install_package(&p.name, &p.version, &p.blake3)?;

            // NOTE: The original code used 4 args for add_history?
            // db.rs shows 5 args.
            // Original install.rs used 5 args.
            // Original use.rs used 5 args: (&p.name, "switch", None, Some(&p.version), true)
            db.add_history(&p.name, "switch", None, Some(&p.version), true)?;

            let output = Output::new();
            output.done(&p.name, &p.version, "switched", None);
        }
        None => {
            // Check if package is installed at all
            let versions = db.list_package_versions(name)?;
            if versions.is_empty() {
                bail!("Package '{name}' is not installed.");
            } else {
                let available = versions
                    .iter()
                    .map(|v| v.version.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                bail!(
                    "Version '{version}' of '{name}' is not installed.\nInstalled versions: {available}"
                );
            }
        }
    }

    Ok(())
}
