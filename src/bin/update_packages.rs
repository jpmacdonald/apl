//! Tool to update packages automatically via GitHub API
//! Usage: cargo run --bin update_packages

use anyhow::Result;
use apl::core::index::PackageIndex;
use regex::Regex;
use reqwest::header;
use serde::Deserialize;
use std::fs;
use std::path::Path;
use toml_edit::{DocumentMut, value};

#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    assets: Vec<GithubAsset>,
}

#[derive(Debug, Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let packages_dir = std::env::current_dir()?.join("packages");
    let output_path = std::env::current_dir()?.join("index.bin");

    // Setup generic client
    let mut headers = header::HeaderMap::new();
    headers.insert(
        header::USER_AGENT,
        header::HeaderValue::from_static("apl-updater"),
    );

    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        println!("🔑 Using GITHUB_TOKEN for authentication");
        headers.insert(
            header::AUTHORIZATION,
            header::HeaderValue::from_str(&format!("Bearer {}", token))?,
        );
    } else {
        println!("⚠️  No GITHUB_TOKEN found. Rate limits may apply.");
    }

    let client = reqwest::Client::builder()
        .default_headers(headers)
        .build()?;

    println!("📦 Updating packages in {}...", packages_dir.display());

    // 1. Iterate over all TOML files
    let mut updated_count = 0;
    for entry in fs::read_dir(&packages_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().is_some_and(|e| e == "toml") {
            match process_package(&client, &path).await {
                Ok(updated) => {
                    if updated {
                        updated_count += 1;
                    }
                }
                Err(e) => {
                    // Don't fail the whole build for one package error
                    eprintln!("❌ Failed to process {}: {}", path.display(), e);
                }
            }
        }
    }

    // 2. Generate Index
    println!("\n📚 Regenerating index at {}...", output_path.display());
    let index = PackageIndex::generate_from_dir(&packages_dir)?;
    index.save_compressed(&output_path)?;

    println!("✅ Done! Updated {} packages.", updated_count);
    Ok(())
}

async fn process_package(client: &reqwest::Client, path: &Path) -> Result<bool> {
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
        // No URL found, skip
        return Ok(false);
    };

    let repo_re = Regex::new(r"github\.com/([^/]+)/([^/]+)")?;
    let captures = if let Some(c) = repo_re.captures(url) {
        c
    } else {
        // Not a GitHub URL
        return Ok(false);
    };

    let owner = &captures[1];
    let repo = &captures[2];

    // Fetch latest release
    // Remove .git if present in repo name (sometimes happens)
    let repo_clean = repo.trim_end_matches(".git");
    let api_url = format!(
        "https://api.github.com/repos/{}/{}/releases/latest",
        owner, repo_clean
    );

    println!("   Checking {} ({}/{})...", name, owner, repo_clean);

    let resp = client.get(&api_url).send().await;
    if let Err(e) = &resp {
        println!("      ⚠️  API check failed: {}", e);
        return Ok(false);
    }
    let resp = resp?;

    if !resp.status().is_success() {
        println!("      ⚠️  GitHub API {} - {}", resp.status(), api_url);
        return Ok(false);
    }

    let release: GithubRelease = resp.json().await?;

    // Parse versions (handle 'v' prefix)
    let latest_tag = release.tag_name.trim_start_matches('v');

    // Very simple version comparison (string equality)
    // In future use semver crate
    if latest_tag == current_version {
        // Up to date
        return Ok(false);
    }

    println!(
        "      ✨ New version found: {} -> {}",
        current_version, latest_tag
    );

    // Find compatible asset
    // Priority: aarch64-apple-darwin -> aarch64-macos -> universal-apple-darwin
    let target_patterns = [
        "aarch64-apple-darwin",
        "aarch64-macos",
        "arm64-mac",
        "universal-apple-darwin",
        "aarch64-unknown-linux-gnu", // Fallback example, but we probably shouldn't
    ];

    let mut asset_url = None;
    for pattern in &target_patterns {
        if let Some(asset) = release.assets.iter().find(|a| {
            a.name.contains(pattern) && (a.name.ends_with(".tar.gz") || a.name.ends_with(".zip"))
        }) {
            asset_url = Some(asset.browser_download_url.clone());
            break;
        }
    }

    let asset_url = if let Some(u) = asset_url {
        u
    } else {
        println!("      ⚠️  No compatible asset found for Darwin ARM64");
        return Ok(false);
    };

    // Download and Hash
    println!("      ⬇️  Downloading {}...", asset_url);
    let bytes = client.get(&asset_url).send().await?.bytes().await?;
    let hash = blake3::hash(&bytes).to_hex().to_string();

    // Update TOML
    doc["package"]["version"] = value(latest_tag);

    // Initial naive approach: update existing URLs.
    // Better approach: if [source] exists, update it. If [binary.arm64] exists, update it.

    if doc.get("source").is_some() {
        doc["source"]["url"] = value(asset_url.clone());
        doc["source"]["blake3"] = value(hash.clone());
    }

    if doc.get("binary").and_then(|b| b.get("arm64")).is_some() {
        doc["binary"]["arm64"]["url"] = value(asset_url);
        doc["binary"]["arm64"]["blake3"] = value(hash);
    }

    fs::write(path, doc.to_string())?;
    println!("      💾 Updated {}", path.display());

    Ok(true)
}
