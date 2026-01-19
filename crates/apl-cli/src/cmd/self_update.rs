//! Self-update command for APL
use crate::ui::Output;
use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;

const APL_REPO_OWNER: &str = "jpmacdonald";
const APL_REPO_NAME: &str = "apl";

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct GithubRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

/// Update APL itself
pub async fn self_update(dry_run: bool) -> Result<()> {
    let output = Output::new();
    let current_version = env!("APL_VERSION");

    output.info("Checking for APL updates via apl.pub...");

    // 1. Fetch index from apl.pub
    let client = Client::new();
    let index_url =
        std::env::var("APL_INDEX_URL").unwrap_or_else(|_| "https://apl.pub/index".to_string());

    let response = client
        .get(&index_url)
        .header("User-Agent", crate::USER_AGENT)
        .send()
        .await
        .context("Failed to fetch index for self-update")?;

    if !response.status().is_success() {
        output.error(&format!(
            "Failed to fetch index: HTTP {}",
            response.status()
        ));
        return Ok(());
    }

    let bytes = response.bytes().await?;
    let decompressed = if bytes.len() >= 4 && bytes[0..4] == crate::ZSTD_MAGIC {
        zstd::decode_all(bytes.as_ref()).context("Failed to decompress index")?
    } else {
        bytes.to_vec()
    };

    let index = crate::index::PackageIndex::from_bytes(&decompressed)
        .context("Failed to parse index for self-update")?;

    // 2. Find 'apl' package
    let entry = match index.find("apl") {
        Some(e) => e,
        None => {
            output.error("Could not find 'apl' in registry. Using GitHub fallback...");
            return self_update_github_fallback(client, dry_run).await;
        }
    };

    let release = entry.latest().context("No releases found for 'apl'")?;
    let latest_version = &release.version;

    // Compare versions
    if !apl_schema::version::is_newer(current_version, latest_version) {
        output.success(&format!("APL is already up to date (v{current_version})"));
        return Ok(());
    }

    output.warning(&format!(
        "Update available: {current_version} -> {latest_version}"
    ));

    if dry_run {
        output.info("Dry run, not installing update.");
        return Ok(());
    }

    // 3. Select binary for current arch
    let arch = apl_schema::Arch::current();
    let binary = release
        .binaries
        .iter()
        .find(|b| b.arch == arch || b.arch == apl_schema::Arch::Universal)
        .context("No compatible binary found for your platform")?;

    // Prefer mirror URL (CAS)
    let download_url = index
        .mirror_base_url
        .as_ref()
        .map(|base| format!("{}/cas/{}", base, binary.hash))
        .unwrap_or_else(|| binary.url.clone());

    output.info(&format!("Downloading from {download_url}..."));

    // Download the binary to a temporary file
    let tmp_dir = tempfile::tempdir().context("Failed to create temporary directory")?;
    let filename = crate::filename_from_url(&binary.url);
    let download_path = tmp_dir.path().join(filename);

    let bytes = client.get(&download_url).send().await?.bytes().await?;
    if bytes.starts_with(b"Artifact not found") {
        return Err(anyhow::anyhow!(
            "Server returned 'Artifact not found' despite 200 OK status. This indicates a missing asset on the CAS server."
        ));
    }
    std::fs::write(&download_path, &bytes).context("Failed to write downloaded asset")?;

    // Extract the archive
    let extract_dir = tmp_dir.path().join("extract");
    let file_size = std::fs::metadata(&download_path).map(|m| m.len()).ok();
    let pkg_name = apl_schema::types::PackageName::from("apl");
    let version_wrapped = apl_schema::types::Version::from(latest_version.as_str());

    let extracted_files = apl_core::io::extract::extract_auto(
        &download_path,
        &extract_dir,
        &output,
        &pkg_name,
        &version_wrapped,
        file_size,
    )
    .context("Failed to extract update archive")?;

    // Find the 'apl' binary in extracted files
    let apl_path = extracted_files
        .iter()
        .find(|f| f.relative_path.file_name().and_then(|s| s.to_str()) == Some("apl"))
        .context("Could not find 'apl' binary in the update archive")?
        .absolute_path
        .clone();

    // Determine install location
    let apl_bin = apl_core::paths::apl_home().join("bin").join("apl");

    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&apl_path, std::fs::Permissions::from_mode(0o755))?;
    }

    // Atomic replace
    // On some systems, we might need to move to a temp file on the same mount point first
    let temp_replace = apl_bin.with_extension("new");
    std::fs::copy(&apl_path, &temp_replace).context("Failed to prepare update binary")?;
    std::fs::rename(&temp_replace, &apl_bin).context("Failed to replace APL binary")?;

    output.success(&format!("APL has been updated to v{latest_version}"));
    output.info("Restart your shell to use the new version.");

    Ok(())
}

/// Fallback to GitHub API if apl.pub is down or "apl" is missing from registry.
async fn self_update_github_fallback(client: Client, dry_run: bool) -> Result<()> {
    let output = Output::new();
    let current_version = env!("APL_VERSION");
    let url = format!("https://api.github.com/repos/{APL_REPO_OWNER}/{APL_REPO_NAME}/releases");

    let response = client
        .get(&url)
        .header("User-Agent", "apl")
        .send()
        .await
        .context("Failed to check for updates on GitHub")?;

    if !response.status().is_success() {
        output.error(&format!(
            "Failed to check for updates on GitHub: HTTP {}",
            response.status()
        ));
        return Ok(());
    }

    let releases: Vec<GithubRelease> = response
        .json()
        .await
        .context("Failed to parse GitHub release info")?;

    let release = releases
        .iter()
        .find(|r| r.tag_name.starts_with('v') && r.tag_name != "index" && !r.draft && !r.prerelease)
        .or_else(|| releases.iter().find(|r| r.tag_name.starts_with('v')));

    let release = match release {
        Some(r) => r,
        None => {
            output.success(&format!("APL is up to date (v{current_version})"));
            return Ok(());
        }
    };

    let latest_version = release.tag_name.trim_start_matches('v');
    if !apl_schema::version::is_newer(current_version, latest_version) {
        output.success(&format!("APL is already up to date (v{current_version})"));
        return Ok(());
    }

    output.warning(&format!(
        "Update available on GitHub: v{current_version} -> v{latest_version}"
    ));

    if dry_run {
        output.info("Dry run, not installing.");
        return Ok(());
    }

    // Since we're in fallback, we just point the user to the binary or we could implement the full
    // download/extract logic here. For brevity and safety, let's keep it minimal for now.
    output.info("Please download the latest release from GitHub: https://github.com/jpmacdonald/apl/releases");
    Ok(())
}
