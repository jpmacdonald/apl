use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;

use crate::types::Sha256Digest;

/// Represents a release found in a remote source
#[derive(Debug, Clone)]
pub struct ReleaseInfo {
    pub tag_name: String,
    pub assets: Vec<AssetInfo>,
    pub prune: bool,
    pub body: String,
    pub prerelease: bool,
}

/// Represents an asset attached to a release
#[derive(Debug, Clone)]
pub struct AssetInfo {
    pub name: String,
    pub download_url: String,
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
