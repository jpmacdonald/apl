use async_trait::async_trait;
use apl_types::Artifact;
use anyhow::Result;

#[async_trait]
pub trait Strategy: Send + Sync {
    /// Fetch artifacts based on the configuration strategy.
    async fn fetch_artifacts(&self) -> Result<Vec<Artifact>>;
}
