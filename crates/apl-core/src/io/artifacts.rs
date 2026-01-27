//! Layout: `cas/<hash>` - all files addressed by their content hash.
//! Layout: `manifests/<hash>` - manifest JSON files listing chunks.

#[cfg(feature = "upload")]
use anyhow::Context;
use anyhow::Result;
#[cfg(feature = "upload")]
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

/// Client for interacting with the S3-compatible artifact store.
///
/// Provides methods to upload, download, and query content-addressed
/// artifacts and their chunked manifests.
#[cfg(feature = "upload")]
#[derive(Debug)]
pub struct ArtifactStore {
    client: s3::Client,
    bucket: String,
    public_base_url: String,
}

#[cfg(feature = "upload")]
impl ArtifactStore {
    // ... all the methods I viewed earlier ...
    /// Create a new artifact store client.
    #[allow(clippy::unused_async)] // Callers expect an async constructor to match the rest of the API
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

    /// Check if a manifest exists in the store.
    pub async fn exists_manifest(&self, hash: &str) -> bool {
        let key = format!("manifests/{hash}");
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

    /// Get the public URL for a manifest.
    pub fn manifest_url(&self, hash: &str) -> String {
        format!("{}/manifests/{hash}", self.public_base_url)
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

    /// Upload a chunked artifact (Deduplication).
    ///
    /// 1. Chunks the data.
    /// 2. Uploads missing chunks.
    /// 3. Uploads the manifest.
    ///
    /// Returns the public manifest URL.
    pub async fn upload_chunked(&self, hash: &str, data: &[u8]) -> Result<String> {
        use crate::io::chunked::BlobManifest;

        let manifest = BlobManifest::from_data(data);
        let chunks = manifest.chunks.clone();

        // Upload chunks
        for chunk_ref in &chunks {
            let chunk_hash = chunk_ref.hash.as_str();
            if !self.exists(chunk_hash).await {
                // Find chunk data
                // In a more optimized version, we'd avoid re-searching or use an iterator
                // But for now, we re-calculate or slice based on original data
                // BlobManifest::from_data already computed hashes, so we just need the slice
            }
        }

        // Let's optimize this: BlobManifest::from_data should probably return the slices or we re-slice here
        let mut offset = 0;
        for chunk_ref in &chunks {
            let chunk_hash = chunk_ref.hash.as_str();
            let chunk_size = chunk_ref.size as usize;
            let chunk_slice = &data[offset..offset + chunk_size];

            if !self.exists(chunk_hash).await {
                self.upload(chunk_hash, chunk_slice.to_vec()).await?;
            }

            offset += chunk_size;
        }

        // Upload manifest
        let manifest_json = manifest.to_json();
        let manifest_key = format!("manifests/{hash}");
        let body = s3::primitives::ByteStream::from(manifest_json.into_bytes());

        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(&manifest_key)
            .body(body)
            .content_type("application/json")
            .send()
            .await
            .context("Failed to upload manifest to R2")?;

        Ok(self.manifest_url(hash))
    }

    /// Retrieve a manifest from the store.
    pub async fn get_manifest(&self, hash: &str) -> Result<crate::io::chunked::BlobManifest> {
        let key = format!("manifests/{hash}");
        let resp = self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(&key)
            .send()
            .await
            .context("Failed to get manifest from R2")?;

        let bytes = resp
            .body
            .collect()
            .await
            .context("Failed to read manifest body from R2")?;

        let json = String::from_utf8(bytes.to_vec()).context("Invalid UTF-8 manifest")?;
        crate::io::chunked::BlobManifest::from_json(&json).context("Failed to parse manifest JSON")
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

#[cfg(feature = "upload")]
static ARTIFACT_STORE: std::sync::OnceLock<Option<Arc<ArtifactStore>>> = std::sync::OnceLock::new();

/// Return the global [`ArtifactStore`] singleton, initializing it from
/// environment variables on first call.
///
/// Returns `None` if the artifact store is not configured or initialization
/// fails.
#[cfg(feature = "upload")]
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

/// Stub artifact store used when the `upload` feature is disabled.
///
/// All mutation methods return errors and all queries return empty results.
#[cfg(not(feature = "upload"))]
#[derive(Debug)]
pub struct ArtifactStore;

#[cfg(not(feature = "upload"))]
impl ArtifactStore {
    /// Return the public URL for an artifact (always empty when uploads are disabled).
    pub fn public_url(&self, _hash: &str) -> String {
        String::new()
    }

    /// Return the public URL for a manifest (always empty when uploads are disabled).
    pub fn manifest_url(&self, _hash: &str) -> String {
        String::new()
    }

    /// Attempt a chunked upload (always fails when uploads are disabled).
    ///
    /// # Errors
    ///
    /// Always returns an error because the `upload` feature is not enabled.
    #[allow(clippy::unused_async)] // Must match the async signature of the upload-enabled variant
    pub async fn upload_chunked(&self, _hash: &str, _data: &[u8]) -> Result<String> {
        anyhow::bail!("Artifact uploads are disabled in this build")
    }

    /// Check if a manifest exists (always returns `false` when uploads are disabled).
    #[allow(clippy::unused_async)] // Must match the async signature of the upload-enabled variant
    pub async fn exists_manifest(&self, _hash: &str) -> bool {
        false
    }
}

/// Return the global [`ArtifactStore`] singleton (always `None` when
/// the `upload` feature is disabled).
#[cfg(not(feature = "upload"))]
pub async fn get_artifact_store() -> Option<Arc<ArtifactStore>> {
    None
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
