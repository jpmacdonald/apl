//! Run command - transient execution without global install

use anyhow::{Context, Result};
use reqwest::Client;
use apl::io::output::InstallOutput;

use apl::apl_home;
use crate::cmd::install::prepare_download_mp;

/// Run a package transiently without global installation
pub async fn run(pkg_name: &str, args: &[String], _dry_run: bool) -> Result<()> {
    let client = Client::new();

    // 1. Resolve and download
    let _output = InstallOutput::new(false);
    let index_path = apl_home().join("index.bin");
    let index = apl::index::PackageIndex::load(&index_path).ok();

    let prepared = prepare_download_mp(&client, pkg_name, None, false, None, index.as_ref(), &_output).await?
        .context(format!("Could not find or download package '{}'", pkg_name))?;

    // 2. Extract to temp (we keep _temp_dir alive to preserve the files)
    let extract_dir = prepared.download_path.parent().unwrap().join("extracted");
    let extracted = apl::extractor::extract_auto(&prepared.download_path, &extract_dir)?;
    
    // Identify the binary to run (first in bin_list or package name)
    let bin_name = prepared.bin_list.first().cloned().unwrap_or_else(|| prepared.name.clone());
    
    let is_raw = extracted.len() == 1 && 
        apl::extractor::detect_format(&prepared.download_path) == apl::extractor::ArchiveFormat::RawBinary;

    // Find the binary path directly in the extracted files (no CAS)
    let bin_path = extracted.iter()
        .find(|f| {
            if is_raw { return true; }
            let fname = f.relative_path.file_name().unwrap().to_string_lossy();
            fname == bin_name || f.relative_path.to_string_lossy() == bin_name
        })
        .map(|f| f.absolute_path.clone())
        .context(format!("Could not find binary '{}' in package archive", bin_name))?;

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
