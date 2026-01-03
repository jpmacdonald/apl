use crate::core::index::HashType;
use anyhow::Result;
use std::collections::HashMap;
use std::fs;

/// Simple persistent hash cache to avoid re-downloading thousands of versions
#[derive(serde::Serialize, serde::Deserialize)]
pub struct CachedEntry {
    pub hash: String,
    pub hash_type: HashType,
    pub timestamp: u64,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
pub struct HashCache {
    /// Map of URL -> CachedEntry (hash + type + timestamp)
    pub entries: HashMap<String, CachedEntry>,
}

impl HashCache {
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

    pub fn save(&self) -> Result<()> {
        let cache_dir = crate::apl_home().join("cache");
        fs::create_dir_all(&cache_dir)?;
        let path = cache_dir.join("hashes.json");
        let content = serde_json::to_string_pretty(self)?;
        fs::write(path, content)?;
        Ok(())
    }

    pub fn get(&self, url: &str) -> Option<(String, HashType)> {
        self.entries.get(url).map(|e| (e.hash.clone(), e.hash_type))
    }

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
