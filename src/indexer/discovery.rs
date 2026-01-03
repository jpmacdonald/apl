use crate::package::{DiscoveryConfig, Package};
use crate::registry::github;
use crate::types::Sha256Digest;
use anyhow::Result;
use std::collections::HashMap;

pub async fn resolve_digest_from_github(
    client: &reqwest::Client,
    release: &github::GithubRelease,
    asset_filename: &str,
) -> Result<Sha256Digest> {
    // Priority 1: Check if the asset itself has a digest field (already validated at deserialization)
    if let Some(asset) = release.assets.iter().find(|a| a.name == asset_filename) {
        if let Some(digest) = &asset.digest {
            return Ok(digest.clone());
        }
    }

    // Look for checksum assets in the release
    for asset in &release.assets {
        let name = asset.name.to_lowercase();
        if name.contains("checksum") || name.contains("sha256") || name.ends_with(".intoto.jsonl") {
            let download_url = &asset.browser_download_url;
            if !download_url.is_empty() {
                // Try to fetch and parse this checksum file
                let resp = client.get(download_url).send().await?;
                if resp.status().is_success() {
                    let text = resp.text().await?;
                    // Search for the target filename in the text
                    if let Some(hash) = scan_text_for_hash(&text, asset_filename) {
                        return Sha256Digest::new(hash);
                    }

                    // Specific handling for JSON/JSONL (e.g. SLSA provenance)
                    if name.ends_with(".json") || name.ends_with(".jsonl") {
                        // Look for the target filename and a 64-char hex string nearby
                        if text.contains(asset_filename) {
                            // Try to find a sha256 pattern
                            let re = regex::Regex::new(r#"[0-9a-fA-F]{64}"#)?;
                            if let Some(m) = re.find(&text) {
                                // This is a bit greedy but works for single-subject JSONs
                                return Sha256Digest::new(m.as_str().to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    // Fallback: Check release body
    if !release.body.is_empty() {
        if let Some(hash) = scan_text_for_hash(&release.body, asset_filename) {
            return Sha256Digest::new(hash);
        }
    }

    anyhow::bail!(
        "Digest for asset '{}' not found in release {}",
        asset_filename,
        release.tag_name
    )
}

fn scan_text_for_hash(text: &str, asset_filename: &str) -> Option<String> {
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            let hash = parts[0];
            let file = parts[1].trim_start_matches('*');
            if file == asset_filename || file.ends_with(asset_filename) {
                // Ensure it looks like a hash (hex, length 40, 64, or 128)
                if hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Some(hash.to_string());
                }
            }
        }
    }

    // Scan for JSON style (SLSA) if applicable
    if asset_filename.ends_with(".json") || asset_filename.ends_with(".jsonl") {
        if text.contains(asset_filename) {
            // Try to find a sha256 pattern nearby?
            // This is harder in raw body without context.
            // For now, simple text scan is best for standard checksum files.
        }
    }

    None
}

pub async fn discover_versions(
    client: &reqwest::Client,
    discovery: &DiscoveryConfig,
) -> Result<Vec<String>> {
    match discovery {
        DiscoveryConfig::GitHub {
            github,
            tag_pattern,
            semver_only,
            include_prereleases,
        } => {
            let repo_ref = crate::types::GitHubRepo::new(github).map_err(|e| anyhow::anyhow!(e))?;
            let owner = repo_ref.owner();
            let repo = repo_ref.name();

            let releases = github::fetch_all_releases(client, owner, repo).await?;

            let mut versions = Vec::new();
            for release in releases {
                if !include_prereleases && release.prerelease {
                    continue;
                }

                let version = extract_version_from_tag(&release.tag_name, tag_pattern);

                if *semver_only && semver::Version::parse(&version).is_err() {
                    continue;
                }

                versions.push(version);
            }

            Ok(versions)
        }
        DiscoveryConfig::Manual { manual } => Ok(manual.clone()),
    }
}

pub fn extract_version_from_tag(tag: &str, pattern: &str) -> String {
    if pattern == "{{version}}" {
        tag.strip_prefix('v').unwrap_or(tag).to_string()
    } else {
        tag.replace(&pattern.replace("{{version}}", ""), "")
    }
}

pub fn guess_github_repo(url: &str) -> Option<String> {
    if url.contains("github.com") {
        let parts: Vec<&str> = url.split('/').collect();
        if parts.len() >= 5 {
            return Some(format!("{}/{}", parts[3], parts[4]));
        }
    }
    None
}

pub fn guess_url_template(url: &str, version: &str, _repo: &str) -> String {
    url.replace(version, "{{version}}")
}

pub fn guess_targets(pkg: &Package) -> Option<HashMap<String, String>> {
    let mut targets = HashMap::new();
    for (arch, binary) in &pkg.targets {
        let arch_name = arch.as_str();
        // Deduce target string from URL
        // Search for the arch in the filename
        let filename = crate::filename_from_url(&binary.url);
        if filename.contains("aarch64") {
            targets.insert(arch_name.to_string(), "aarch64".to_string());
        } else if filename.contains("x86_64") {
            targets.insert(arch_name.to_string(), "x86_64".to_string());
        }
    }

    if targets.is_empty() {
        None
    } else {
        Some(targets)
    }
}
