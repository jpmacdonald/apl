use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;

use crate::types::Sha256Digest;

/// Represents a release found in a remote source (e.g. a GitHub release or a
/// ports index entry).
#[derive(Debug, Clone)]
pub struct ReleaseInfo {
    /// The Git tag name associated with this release (e.g. `"v1.2.3"`).
    pub tag_name: String,
    /// Downloadable assets attached to this release.
    pub assets: Vec<AssetInfo>,
    /// Whether this release should be pruned (e.g. drafts).
    pub prune: bool,
    /// The release body / description text.
    pub body: String,
    /// Whether this release is marked as a pre-release.
    pub prerelease: bool,
}

/// Represents a downloadable asset attached to a release.
#[derive(Debug, Clone)]
pub struct AssetInfo {
    /// Filename of the asset (e.g. `"myapp-v1.0.0-arm64-macos.tar.gz"`).
    pub name: String,
    /// Direct download URL for the asset.
    pub download_url: String,
    /// Optional pre-computed `SHA-256` digest, when provided by the forge.
    pub digest: Option<Sha256Digest>,
}

/// A remote source that can list available releases (e.g. GitHub, GitLab)
#[async_trait]
pub trait ListingSource: Send + Sync {
    /// Unique identifier for this source instance (e.g. "github:owner/repo")
    fn key(&self) -> String;

    /// Fetch all valid releases from this source
    async fn fetch_releases(&self, client: &Client) -> Result<Vec<ReleaseInfo>>;
}
