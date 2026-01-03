//! Run command - transient execution without global install

use anyhow::{Context, Result};
use reqwest::Client;

use apl::apl_home;
use apl::ops::flow::UnresolvedPackage;
use apl::types::PackageName;
use apl::ui::Output;

/// Run a package transiently without global installation
pub async fn run(pkg_name: &str, args: &[String], _dry_run: bool) -> Result<()> {
    let client = Client::new();

    // 1. Resolve and download
    let output = Output::new();
    let index_path = apl_home().join("index.bin");
    let index = apl::core::index::PackageIndex::load(&index_path).ok();

    let pkg_name_new = PackageName::from(pkg_name);
    let unresolved = UnresolvedPackage::new(pkg_name_new, None);
    let resolved = unresolved.resolve(index.as_ref())?;
    let prepared = resolved.prepare(&client, &output).await?;

    // 2. Already Extracted (by prepare_download_new)
    let extract_dir = prepared.extracted_path;

    // Identify the binary to run (first in bin_list or package name)
    let bin_name = prepared
        .bin_list
        .first()
        .cloned()
        .unwrap_or_else(|| prepared.resolved.name.to_string());

    // Find the binary path in the extracted files
    let bin_path = walkdir::WalkDir::new(&extract_dir)
        .into_iter()
        .flatten()
        .find(|entry| {
            if !entry.file_type().is_file() {
                return false;
            }
            let fname = entry.file_name().to_string_lossy();
            if fname == bin_name {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(meta) = entry.metadata() {
                        return meta.permissions().mode() & 0o111 != 0;
                    }
                }
                return true;
            }
            false
        })
        .map(|e| e.path().to_owned())
        .ok_or_else(|| anyhow::anyhow!("Could not find binary '{bin_name}' in package archive"))?;

    // 3. Ensure executable and run
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&bin_path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&bin_path, perms)?;
    }

    let status = std::process::Command::new(&bin_path)
        .args(args)
        .status()
        .context("Failed to execute process")?;

    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }

    Ok(())
}
