use anyhow::Result;
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
    "macos-aarch64", // fastfetch pattern
    "osx-arm64",
    "osx_arm64",
    "aarch64-macos",
    "aarch64-mac",
    "arm64-macos",
    "arm64-mac",
    "-aarch64", // generic aarch64 (careful - last resort)
];

/// Priority patterns for macOS x86_64 binaries
pub const MACOS_X86_PATTERNS: &[&str] = &[
    "x86_64-apple-darwin",
    "amd64-apple-darwin",
    "darwin-x86_64",
    "darwin_x86_64",
    "darwin-x64", // pulumi pattern
    "darwin-amd64",
    "darwin_amd64",
    "macos-x86_64",
    "macos_x86_64",
    "macOS-x86_64",
    "macos-amd64", // fastfetch pattern
    "osx-x86_64",
    "osx-x64",
    "x64-mac",
    "-amd64",  // generic amd64 (careful - last resort)
    "-x86_64", // generic x86_64 (careful - last resort)
];

/// Universal binary patterns (work on both ARM64 and x86_64)
pub const MACOS_UNIVERSAL_PATTERNS: &[&str] = &["universal", "macos", "mac"];

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

/// Find both ARM64 and x86_64 assets for macOS
pub fn find_macos_assets<'a>(
    release: &'a GithubRelease,
    package_name: &str,
) -> (Option<&'a GithubAsset>, Option<&'a GithubAsset>) {
    let arm64_asset = find_asset_for_arch(release, package_name, MACOS_ARM_PATTERNS);
    let x86_asset = find_asset_for_arch(release, package_name, MACOS_X86_PATTERNS);

    // If we didn't find arch-specific, try universal binaries
    let arm64_final = arm64_asset
        .or_else(|| find_asset_for_arch(release, package_name, MACOS_UNIVERSAL_PATTERNS));
    let x86_final =
        x86_asset.or_else(|| find_asset_for_arch(release, package_name, MACOS_UNIVERSAL_PATTERNS));

    (arm64_final, x86_final)
}

/// Find asset matching specific architecture patterns
fn find_asset_for_arch<'a>(
    release: &'a GithubRelease,
    package_name: &str,
    patterns: &[&str],
) -> Option<&'a GithubAsset> {
    let package_name_low = package_name.to_lowercase();
    for pattern in patterns {
        if let Some(asset) = release.assets.iter().find(|a| {
            let name = a.name.to_lowercase();
            let pat = pattern.to_lowercase();

            if name.contains(&pat) {
                // archives
                if name.ends_with(".tar.gz")
                    || name.ends_with(".zip")
                    || name.ends_with(".tar.xz")
                    || name.ends_with(".tar.zst")
                    || name.ends_with(".tzst")
                    || name.ends_with(".dmg")
                    || name.ends_with(".pkg")
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
            return Some(asset);
        }
    }

    // Fallback for macOS apps that just name their DMG/PKG after the app
    release.assets.iter().find(|a| {
        let name = a.name.to_lowercase();
        (name.ends_with(".dmg") || name.ends_with(".pkg")) && name.contains(&package_name_low)
    })
}

/// Legacy function - find best ARM64 asset (kept for compatibility)
pub fn find_best_asset<'a>(
    release: &'a GithubRelease,
    package_name: &str,
) -> Option<(&'a GithubAsset, bool)> {
    let package_name_low = package_name.to_lowercase();
    for pattern in MACOS_ARM_PATTERNS {
        if let Some(asset) = release.assets.iter().find(|a| {
            let name = a.name.to_lowercase();
            let pat = pattern.to_lowercase();

            if name.contains(&pat) {
                // archives
                if name.ends_with(".tar.gz")
                    || name.ends_with(".zip")
                    || name.ends_with(".tar.xz")
                    || name.ends_with(".tar.zst")
                    || name.ends_with(".tzst")
                    || name.ends_with(".dmg")
                    || name.ends_with(".pkg")
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
                || asset.name.ends_with(".tar.xz")
                || asset.name.ends_with(".tar.zst")
                || asset.name.ends_with(".tzst")
                || asset.name.ends_with(".dmg")
                || asset.name.ends_with(".pkg");
            return Some((asset, is_archive));
        }
    }

    // NEW: Fallback for macOS apps that just name their DMG/PKG after the app
    // e.g. Alacritty-v0.16.1.dmg
    if let Some(asset) = release.assets.iter().find(|a| {
        let name = a.name.to_lowercase();
        (name.ends_with(".dmg") || name.ends_with(".pkg")) && name.contains(&package_name_low)
    }) {
        return Some((asset, true));
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
    let latest_tag_raw = &release.tag_name;
    let latest_tag = strip_tag_prefix(latest_tag_raw, name);

    if latest_tag == current_version {
        return Ok(false);
    }

    println!(
        "      New version found: {} -> {}",
        current_version, latest_tag
    );

    let asset_discovery = find_best_asset(&release, name);

    if let Some((asset, _is_archive)) = asset_discovery {
        println!("      Downloading {}...", asset.browser_download_url);
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
    } else {
        // Fallback for source-only or custom binary updates
        // If we have a [source] section with an archive URL, update it
        if let Some(src_url) = doc
            .get("source")
            .and_then(|s| s.get("url"))
            .and_then(|u| u.as_str())
        {
            if src_url.contains("/archive/refs/tags/") {
                println!("      Source-only update detected (no binary asset found)");
                let new_url = src_url
                    .replace(&current_version, &latest_tag)
                    .replace(&release.tag_name, &latest_tag_raw); // just in case

                println!("      Downloading source archive {}...", new_url);
                let bytes = client.get(&new_url).send().await?.bytes().await?;
                let hash = blake3::hash(&bytes).to_hex().to_string();

                doc["package"]["version"] = value(latest_tag);
                doc["source"]["url"] = value(new_url);
                doc["source"]["blake3"] = value(hash);
            } else {
                anyhow::bail!(
                    "No compatible asset found for Darwin ARM64 and source URL is not a standard tag archive"
                );
            }
        } else {
            anyhow::bail!(
                "No compatible asset found for Darwin ARM64 and no source section found to update"
            );
        }
    }

    fs::write(path, doc.to_string())?;
    println!("      Updated {}", path.display());

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
        if resp.status() == 404 {
            // Fallback 1: fetch all releases and pick the most recent one
            let releases_url = format!("https://api.github.com/repos/{}/{}/releases", owner, repo);
            let releases_resp = client.get(&releases_url).send().await?;
            if releases_resp.status().is_success() {
                let releases: Vec<GithubRelease> = releases_resp.json().await?;
                if let Some(first) = releases.into_iter().next() {
                    return Ok(first);
                }
            }

            // Fallback 2: fetch all tags and pick the most recent one (for repos without formal releases)
            let tags_url = format!("https://api.github.com/repos/{}/{}/tags", owner, repo);
            let tags_resp = client.get(&tags_url).send().await?;
            if tags_resp.status().is_success() {
                #[derive(Deserialize)]
                struct GithubTag {
                    name: String,
                }
                let tags: Vec<GithubTag> = tags_resp.json().await?;
                if let Some(first) = tags.into_iter().next() {
                    return Ok(GithubRelease {
                        tag_name: first.name,
                        assets: vec![], // No assets for a tag
                    });
                }
            }
        }
        anyhow::bail!("GitHub API error: {} for {}", resp.status(), url);
    }

    Ok(resp.json().await?)
}
