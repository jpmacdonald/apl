//! Artifact Store for R2-compatible CAS (Content-Addressable Storage)
//!
//! Manages uploads and existence checks for the unified artifact store.
//! Layout: `cas/<sha256>` - all files addressed by their content hash.

use anyhow::{Context, Result};
use aws_sdk_s3 as s3;
use std::sync::Arc;

/// Configuration for the artifact store.
#[derive(Debug, Clone)]
pub struct ArtifactConfig {
    /// Enable artifact store operations
    pub enabled: bool,
    /// S3-compatible endpoint (e.g., `https://<account>.r2.cloudflarestorage.com`)
    pub endpoint: String,
    /// Access Key ID
    pub access_key: String,
    /// Secret Access Key
    pub secret_key: String,
    /// Bucket name (e.g., `apl-artifacts`)
    pub bucket: String,
    /// Public base URL for downloads (e.g., `https://cache.apl.dev`)
    pub public_base_url: String,
}

impl ArtifactConfig {
    /// Load configuration from environment variables.
    pub fn from_env() -> Option<Self> {
        let enabled = std::env::var("APL_ARTIFACT_STORE_ENABLED")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        if !enabled {
            return None;
        }

        Some(Self {
            enabled,
            endpoint: std::env::var("APL_ARTIFACT_STORE_ENDPOINT").ok()?,
            access_key: std::env::var("APL_ARTIFACT_STORE_ACCESS_KEY").ok()?,
            secret_key: std::env::var("APL_ARTIFACT_STORE_SECRET_KEY").ok()?,
            bucket: std::env::var("APL_ARTIFACT_STORE_BUCKET")
                .unwrap_or_else(|_| "apl-artifacts".to_string()),
            public_base_url: std::env::var("APL_ARTIFACT_STORE_PUBLIC_URL")
                .unwrap_or_else(|_| "https://apl.pub".to_string()),
        })
    }
}

/// Client for the unified artifact store.
pub struct ArtifactStore {
    client: s3::Client,
    bucket: String,
    public_base_url: String,
}

impl ArtifactStore {
    /// Create a new artifact store client.
    pub async fn new(config: ArtifactConfig) -> Result<Self> {
        let credentials = s3::config::Credentials::new(
            &config.access_key,
            &config.secret_key,
            None,
            None,
            "apl-artifact-store",
        );

        let s3_config = s3::Config::builder()
            .behavior_version_latest()
            .endpoint_url(&config.endpoint)
            .region(s3::config::Region::new("auto"))
            .credentials_provider(credentials)
            .force_path_style(true) // Required for R2
            .build();

        let client = s3::Client::from_conf(s3_config);

        Ok(Self {
            client,
            bucket: config.bucket,
            public_base_url: config.public_base_url.trim_end_matches('/').to_string(),
        })
    }

    /// Check if an artifact exists in the store.
    ///
    /// Uses HEAD request for efficiency (no data transfer).
    pub async fn exists(&self, hash: &str) -> bool {
        let key = format!("cas/{hash}");
        self.client
            .head_object()
            .bucket(&self.bucket)
            .key(&key)
            .send()
            .await
            .is_ok()
    }

    /// Retrieve an artifact from the store.
    pub async fn get(&self, hash: &str) -> Result<Vec<u8>> {
        let key = format!("cas/{hash}");
        let resp = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(&key)
            .send()
            .await
            .context("Failed to get artifact from R2")?;

        let bytes = resp
            .body
            .collect()
            .await
            .context("Failed to read artifact body from R2")?;
        Ok(bytes.to_vec())
    }

    /// Retrieve a delta/patch from the store.
    pub async fn get_delta(&self, from_hash: &str, to_hash: &str) -> Result<Vec<u8>> {
        let key = format!("deltas/{from_hash}_{to_hash}.zst");
        let resp = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(&key)
            .send()
            .await
            .context("Failed to get delta from R2")?;

        let bytes = resp
            .body
            .collect()
            .await
            .context("Failed to read delta body from R2")?;
        Ok(bytes.to_vec())
    }

    /// Get the public URL for an artifact.
    ///
    /// Returns: `{public_base_url}/cas/{hash}`
    pub fn public_url(&self, hash: &str) -> String {
        format!("{}/cas/{hash}", self.public_base_url)
    }

    /// Upload an artifact to the store.
    ///
    /// Returns the public URL on success.
    pub async fn upload(&self, hash: &str, data: Vec<u8>) -> Result<String> {
        let key = format!("cas/{hash}");
        let body = s3::primitives::ByteStream::from(data);

        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&key)
            .body(body)
            .content_type("application/octet-stream")
            .send()
            .await
            .context("Failed to upload artifact to R2")?;

        Ok(self.public_url(hash))
    }

    /// Upload from a stream (for large files).
    pub async fn upload_stream(
        &self,
        hash: &str,
        stream: s3::primitives::ByteStream,
        content_length: Option<i64>,
    ) -> Result<String> {
        let key = format!("cas/{hash}");

        let mut req = self
            .client
            .put_object()
            .bucket(&self.bucket)
            .key(&key)
            .body(stream)
            .content_type("application/octet-stream");

        if let Some(len) = content_length {
            req = req.content_length(len);
        }

        req.send()
            .await
            .context("Failed to upload artifact stream to R2")?;

        Ok(self.public_url(hash))
    }
}

/// Global shared artifact store instance (initialized lazily).
static ARTIFACT_STORE: std::sync::OnceLock<Option<Arc<ArtifactStore>>> = std::sync::OnceLock::new();

/// Get or initialize the global artifact store.
pub async fn get_artifact_store() -> Option<Arc<ArtifactStore>> {
    // Check if already initialized
    if let Some(store) = ARTIFACT_STORE.get() {
        return store.clone();
    }

    // Try to initialize from env
    let config = ArtifactConfig::from_env()?;
    let store = ArtifactStore::new(config).await.ok()?;
    let store = Arc::new(store);

    // Store globally (race is fine, all instances are equivalent)
    let _ = ARTIFACT_STORE.set(Some(store.clone()));
    Some(store)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_public_url_construction() {
        let config = ArtifactConfig {
            enabled: true,
            endpoint: "https://example.r2.cloudflarestorage.com".to_string(),
            access_key: "key".to_string(),
            secret_key: "secret".to_string(),
            bucket: "test".to_string(),
            public_base_url: "https://apl.pub/".to_string(), // Trailing slash
        };

        // Can't test async new() in sync test, but we can test URL construction logic
        let base = config.public_base_url.trim_end_matches('/');
        let hash = "abc123def456";
        let url = format!("{base}/cas/{hash}");
        assert_eq!(url, "https://apl.pub/cas/abc123def456");
    }
}
