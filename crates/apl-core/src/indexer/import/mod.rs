//! Importers for translating external package registries (Homebrew, Nix) to APL TOML.

/// Homebrew formula importer.
pub mod homebrew;

use anyhow::Result;
use std::path::Path;

use crate::package::{AssetConfig, AssetSelector, DiscoveryConfig};
use std::collections::HashMap;

/// Import packages from an external registry into the APL registry directory.
///
/// Currently supports `"homebrew"` as the `source` identifier.
///
/// # Errors
///
/// Returns an error if the `source` is unrecognised, or if any individual
/// package import fails (e.g. network errors, missing metadata).
pub async fn import_packages(source: &str, packages: &[String], registry_dir: &Path) -> Result<()> {
    match source {
        "homebrew" => homebrew::import_homebrew_packages(packages, registry_dir).await,
        _ => anyhow::bail!("Unknown import source: {source}"),
    }
}

/// Analyzes a URL to determine the upstream source repository (Forge).
///
/// This distinguishes between the "Importer Source" (e.g. Homebrew, which provides metadata)
/// and the "Upstream Source" (e.g. GitHub, GitLab, which hosts the binaries).
///
/// # Errors
///
/// Returns an error if the URL does not match any known forge pattern (e.g.
/// a non-GitHub URL).
pub fn analyze_upstream_url(url: &str) -> Result<(DiscoveryConfig, AssetConfig)> {
    // Check for GitHub
    if url.contains("github.com") {
        let re = regex::Regex::new(r"github\.com/([^/]+)/([^/]+)")?;
        if let Some(caps) = re.captures(url) {
            let owner = &caps[1];
            let repo = caps[2].trim_end_matches(".git");
            let source_repo = format!("{owner}/{repo}");

            let discovery = DiscoveryConfig::GitHub {
                github: source_repo.clone(),
                tag_pattern: "{{version}}".to_string(), // Default, user might need to adjust
                include_prereleases: false,
            };

            // Default assets config for typical Rust/Go apps
            let mut select = HashMap::new();
            select.insert(
                "arm64-macos".to_string(),
                AssetSelector::Suffix {
                    suffix: "aarch64-apple-darwin".to_string(),
                },
            );
            select.insert(
                "x86_64-macos".to_string(),
                AssetSelector::Suffix {
                    suffix: "x86_64-apple-darwin".to_string(),
                },
            );

            let assets = AssetConfig {
                select,
                skip_checksums: false,
                checksum_url: None,
            };

            return Ok((discovery, assets));
        }
    }

    // TODO: Add logic to detect GitLab/SourceForge URLs once DiscoveryConfig supports them.
    // Note: GitLab is a Source Forge (where code lives), not a Package Repository (where formulas live).

    // Fallback: Manual (User needs to fill this in)
    anyhow::bail!("Could not automatically detect upstream GitHub repository from URL: {url}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_github_from_release_url() {
        let url = "https://github.com/jqlang/jq/releases/download/jq-1.7.1/jq-1.7.1.tar.gz";
        let result = analyze_upstream_url(url);
        assert!(result.is_ok());

        let (discovery, _assets) = result.unwrap();
        match discovery {
            DiscoveryConfig::GitHub { github, .. } => {
                assert_eq!(github, "jqlang/jq");
            }
            _ => panic!("Expected GitHub discovery config"),
        }
    }

    #[test]
    fn detects_github_from_archive_url() {
        let url = "https://github.com/BurntSushi/ripgrep/archive/refs/tags/14.1.0.tar.gz";
        let result = analyze_upstream_url(url);
        assert!(result.is_ok());

        let (discovery, _) = result.unwrap();
        match discovery {
            DiscoveryConfig::GitHub { github, .. } => {
                assert_eq!(github, "BurntSushi/ripgrep");
            }
            _ => panic!("Expected GitHub discovery config"),
        }
    }

    #[test]
    fn fails_for_non_github_url() {
        let url = "https://example.com/some/package.tar.gz";
        let result = analyze_upstream_url(url);
        assert!(result.is_err());
    }

    #[test]
    fn sets_default_asset_patterns() {
        let url = "https://github.com/sharkdp/fd/releases/download/v10.2.0/fd-v10.2.0.tar.gz";
        let (_, assets) = analyze_upstream_url(url).unwrap();

        assert!(assets.select.contains_key("arm64-macos"));
        assert!(assets.select.contains_key("x86_64-macos"));
    }
}
