pub mod context;
pub mod error;
pub mod flow;
pub mod install;
pub mod remove;
pub mod resolve;
pub mod switch;

pub use context::Context;
pub use error::InstallError;

use crate::bin_path;
use std::path::Path;

/// Shared utility to link binaries from a package store path to the global bin directory.
pub fn link_binaries(
    bin_list: &[String],
    pkg_store_path: &Path,
) -> Result<Vec<(String, String)>, InstallError> {
    let mut files_to_record = Vec::new();
    let mut bins_to_link = Vec::new();

    if !bin_list.is_empty() {
        for bin_spec in bin_list {
            if bin_spec.contains(':') {
                let parts: Vec<&str> = bin_spec.split(':').collect();
                bins_to_link.push((parts[0].to_string(), parts[1].to_string()));
            } else {
                let target = Path::new(bin_spec)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| bin_spec.clone());
                bins_to_link.push((bin_spec.clone(), target));
            }
        }
    } else {
        let bin_dir = pkg_store_path.join("bin");
        let search_dir = if bin_dir.exists() {
            &bin_dir
        } else {
            pkg_store_path
        };
        if let Ok(entries) = std::fs::read_dir(search_dir) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    #[cfg(unix)]
                    if meta.is_file() {
                        use std::os::unix::fs::PermissionsExt;
                        if meta.permissions().mode() & 0o111 != 0 {
                            let name = entry.file_name().to_string_lossy().to_string();
                            bins_to_link.push((name.clone(), name));
                        }
                    }
                }
            }
        }
    }

    for (src_rel, target_name) in bins_to_link {
        let src_path = pkg_store_path.join(&src_rel);
        if !src_path.exists() {
            continue;
        }

        let target = bin_path().join(target_name);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        if target.exists() || target.is_symlink() {
            std::fs::remove_file(&target).ok();
        }

        #[cfg(unix)]
        std::os::unix::fs::symlink(&src_path, &target).map_err(InstallError::Io)?;

        files_to_record.push((target.to_string_lossy().to_string(), "SYMLINK".to_string()));
    }

    Ok(files_to_record)
}
