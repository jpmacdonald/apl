//! Self-update command for APL
use anyhow::{Context, Result};
use apl::ui::Output;
use reqwest::Client;
use serde::Deserialize;

const APL_REPO_OWNER: &str = "jpmacdonald";
const APL_REPO_NAME: &str = "apl";

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

/// Update APL itself
pub async fn self_update(dry_run: bool) -> Result<()> {
    let output = Output::new();
    let current_version = env!("CARGO_PKG_VERSION");

    output.info("Checking for APL updates...");

    // Fetch latest release from GitHub
    // Fetch releases from GitHub
    let client = Client::new();
    let url = format!(
        "https://api.github.com/repos/{}/{}/releases",
        APL_REPO_OWNER, APL_REPO_NAME
    );

    let response = client
        .get(&url)
        .header("User-Agent", "apl")
        .send()
        .await
        .context("Failed to check for updates")?;

    if !response.status().is_success() {
        output.error(&format!(
            "Failed to check for updates: HTTP {}",
            response.status()
        ));
        return Ok(());
    }

    let releases: Vec<GithubRelease> = response
        .json()
        .await
        .context("Failed to parse release info")?;

    // Find the latest release that is a valid version (starts with 'v') and not "index"
    let release = releases
        .iter()
        .find(|r| r.tag_name.starts_with('v') && r.tag_name != "index" && !r.draft && !r.prerelease)
        .or_else(|| {
            // Fallback to prereleases if no stable found
            releases.iter().find(|r| r.tag_name.starts_with('v'))
        });

    let release = match release {
        Some(r) => r,
        None => {
            // No versioned releases found
            output.success(&format!("APL is up to date (v{current_version})"));
            return Ok(());
        }
    };

    // Strip 'v' prefix if present
    let latest_version = release.tag_name.trim_start_matches('v');

    // Compare versions
    if !apl::core::version::is_newer(current_version, latest_version) {
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

    // Find the right asset for current architecture
    let arch = std::env::consts::ARCH;
    let asset = release
        .assets
        .iter()
        .find(|a| {
            let name = a.name.to_lowercase();
            (name.contains("darwin") || name.contains("macos") || name.contains("apple"))
                && (name.contains(arch) || name.contains("arm64") || name.contains("aarch64"))
        })
        .or_else(|| {
            // Fallback: look for universal binary
            release
                .assets
                .iter()
                .find(|a| a.name.to_lowercase().contains("universal"))
        });

    let asset = match asset {
        Some(a) => a,
        None => {
            output.error("No compatible binary found for your platform");
            return Ok(());
        }
    };

    output.info(&format!("Downloading {}...", asset.name));

    // Download the binary
    let bytes = client
        .get(&asset.browser_download_url)
        .send()
        .await?
        .bytes()
        .await?;

    // Determine install location
    let apl_bin = apl::apl_home().join("bin").join("apl");

    // Write to temp file first
    let temp_path = apl_bin.with_extension("new");
    std::fs::write(&temp_path, &bytes)?;

    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&temp_path, std::fs::Permissions::from_mode(0o755))?;
    }

    // Atomic replace
    std::fs::rename(&temp_path, &apl_bin)?;

    output.success(&format!("APL has been updated to v{latest_version}"));
    output.info("Restart your shell to use the new version.");

    output.wait();

    Ok(())
}
