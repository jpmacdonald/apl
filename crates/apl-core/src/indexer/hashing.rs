use anyhow::Result;
use apl_schema::index::HashType;
use std::collections::HashMap;
use std::fs;

/// A single cached hash entry persisted to disk.
///
/// Stores the computed hash value, its algorithm type, and a UNIX timestamp
/// recording when the entry was created.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct CachedEntry {
    /// The hex-encoded hash digest.
    pub hash: String,
    /// The hash algorithm used (e.g. `SHA-256`).
    pub hash_type: HashType,
    /// UNIX epoch timestamp of when this entry was cached.
    pub timestamp: u64,
}

/// Persistent on-disk cache mapping asset URLs to their computed hashes.
///
/// Avoids re-downloading and re-hashing assets that have already been
/// processed in a previous indexing run. The cache is serialized as JSON
/// under `$APL_HOME/cache/hashes.json`.
#[derive(Debug, serde::Serialize, serde::Deserialize, Default)]
pub struct HashCache {
    /// Map of asset URL to its [`CachedEntry`] (hash, type, and timestamp).
    pub entries: HashMap<String, CachedEntry>,
}

impl HashCache {
    /// Load the hash cache from `$APL_HOME/cache/hashes.json`.
    ///
    /// Returns a default (empty) cache if the file does not exist or cannot
    /// be parsed.
    pub fn load() -> Self {
        let path = crate::apl_home().join("cache").join("hashes.json");
        if path.exists() {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(cache) = serde_json::from_str(&content) {
                    return cache;
                }
            }
        }
        Self::default()
    }

    /// Persist the cache to `$APL_HOME/cache/hashes.json`.
    ///
    /// Creates the cache directory if it does not already exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the cache directory cannot be created, the cache
    /// cannot be serialized, or the file cannot be written.
    pub fn save(&self) -> Result<()> {
        let cache_dir = crate::apl_home().join("cache");
        fs::create_dir_all(&cache_dir)?;
        let path = cache_dir.join("hashes.json");
        let content = serde_json::to_string_pretty(self)?;
        fs::write(path, content)?;
        Ok(())
    }

    /// Look up the cached hash for the given asset URL.
    ///
    /// Returns `Some((hash, hash_type))` if the URL has been cached, or
    /// `None` otherwise.
    pub fn get(&self, url: &str) -> Option<(String, HashType)> {
        self.entries.get(url).map(|e| (e.hash.clone(), e.hash_type))
    }

    /// Insert or update a hash entry for the given asset URL.
    ///
    /// # Panics
    ///
    /// Panics if the system clock is set before the UNIX epoch.
    pub fn insert(&mut self, url: String, hash: String, hash_type: HashType) {
        self.entries.insert(
            url,
            CachedEntry {
                hash,
                hash_type,
                timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
            },
        );
    }
}
