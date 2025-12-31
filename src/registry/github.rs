use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct GithubRelease {
    pub tag_name: String,
    pub assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
pub struct GithubAsset {
    pub name: String,
    pub browser_download_url: String,
}

/// Priority patterns for macOS ARM64 binaries
pub const MACOS_ARM_PATTERNS: &[&str] = &[
    "aarch64-apple-darwin",
    "arm64-apple-darwin",
    "darwin-arm64",
    "darwin_arm64",
    "macos-arm64",
    "macos_arm64",
    "macOS-arm64",
    "macOS_arm64",
    "osx-arm64",
    "osx_arm64",
    "aarch64-macos",
    "aarch64-mac",
    "arm64-macos",
    "arm64-mac",
    "universal-apple-darwin",
];

/// Strip common prefixes from GitHub tags (e.g., 'v1.0.0', 'jq-1.8.1' -> '1.8.1')
pub fn strip_tag_prefix(tag: &str, package_name: &str) -> String {
    let mut version = tag;
    let prefixes = [
        format!("{}-", package_name),
        format!("{}_", package_name),
        "v".to_string(),
    ];

    let mut changed = true;
    while changed {
        changed = false;
        for p in &prefixes {
            if version.starts_with(p) {
                version = &version[p.len()..];
                changed = true;
            }
        }
    }
    version.to_string()
}

/// Logic to find the best matching asset in a release
pub fn find_best_asset(release: &GithubRelease) -> Option<(&GithubAsset, bool)> {
    for pattern in MACOS_ARM_PATTERNS {
        if let Some(asset) = release.assets.iter().find(|a| {
            let name = a.name.to_lowercase();
            let pat = pattern.to_lowercase();

            if name.contains(&pat) {
                // archives
                if name.ends_with(".tar.gz") || name.ends_with(".zip") || name.ends_with(".tar.xz")
                {
                    return true;
                }
                // raw binaries (usually no extension or custom suffix)
                if !name.contains('.') || name.ends_with(".exe") {
                    return true;
                }
            }
            false
        }) {
            let is_archive = asset.name.ends_with(".tar.gz")
                || asset.name.ends_with(".zip")
                || asset.name.ends_with(".tar.xz");
            return Some((asset, is_archive));
        }
    }
    None
}

use regex::Regex;
use std::fs;
use std::path::Path;
use toml_edit::{DocumentMut, value};

pub async fn update_package_definition(client: &reqwest::Client, path: &Path) -> Result<bool> {
    let content = fs::read_to_string(path)?;
    let mut doc = content.parse::<DocumentMut>()?;

    let name = doc["package"]["name"].as_str().unwrap_or("unknown");
    let current_version = doc["package"]["version"]
        .as_str()
        .unwrap_or("0.0.0")
        .to_string();

    // Check if we have a GitHub source
    let url = if let Some(u) = doc
        .get("source")
        .and_then(|s| s.get("url"))
        .and_then(|u| u.as_str())
    {
        u
    } else if let Some(u) = doc
        .get("binary")
        .and_then(|b| b.get("arm64"))
        .and_then(|b| b.get("url"))
        .and_then(|u| u.as_str())
    {
        u
    } else {
        return Ok(false);
    };

    let repo_re = Regex::new(r"github\.com/([^/]+)/([^/]+)")?;
    let captures = if let Some(c) = repo_re.captures(url) {
        c
    } else {
        return Ok(false);
    };

    let owner = &captures[1];
    let repo_name = captures[2].trim_end_matches(".git");

    println!("   Checking {} ({}/{})...", name, owner, repo_name);

    let release = fetch_latest_release(client, owner, repo_name).await?;
    let latest_tag = strip_tag_prefix(&release.tag_name, name);

    if latest_tag == current_version {
        return Ok(false);
    }

    println!(
        "      ✨ New version found: {} -> {}",
        current_version, latest_tag
    );

    let (asset, _is_archive) =
        find_best_asset(&release).context("No compatible asset found for Darwin ARM64")?;

    println!("      ⬇️  Downloading {}...", asset.browser_download_url);
    let bytes = client
        .get(&asset.browser_download_url)
        .send()
        .await?
        .bytes()
        .await?;
    let hash = blake3::hash(&bytes).to_hex().to_string();

    // Update TOML
    doc["package"]["version"] = value(latest_tag);

    if doc.get("source").is_some() {
        doc["source"]["url"] = value(asset.browser_download_url.clone());
        doc["source"]["blake3"] = value(hash.clone());
    }

    if doc.get("binary").and_then(|b| b.get("arm64")).is_some() {
        doc["binary"]["arm64"]["url"] = value(asset.browser_download_url.clone());
        doc["binary"]["arm64"]["blake3"] = value(hash);
    }

    fs::write(path, doc.to_string())?;
    println!("      💾 Updated {}", path.display());

    Ok(true)
}

pub async fn fetch_latest_release(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
) -> Result<GithubRelease> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/releases/latest",
        owner, repo
    );
    let resp = client.get(&url).send().await?;

    if !resp.status().is_success() {
        anyhow::bail!("GitHub API error: {} for {}", resp.status(), url);
    }

    Ok(resp.json().await?)
}
