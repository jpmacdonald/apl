//! Binary index using postcard + zstd
//!
//! Compact package registry fetched from CDN.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum IndexError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("Serialization error: {0}")]
    Postcard(#[from] postcard::Error),
}

/// Bottle info in the index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexBottle {
    /// Architecture (e.g., "aarch64-apple-darwin")
    pub arch: String,
    /// Download URL
    pub url: String,
    /// BLAKE3 hash
    pub blake3: String,
}

/// Compact package entry in the index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexEntry {
    /// Package name
    pub name: String,
    /// Latest version
    pub version: String,
    /// Package description
    #[serde(default)]
    pub description: String,
    /// Available bottles with URLs
    pub bottles: Vec<IndexBottle>,
    /// Runtime dependencies (names only)
    #[serde(default)]
    pub deps: Vec<String>,
    /// Binary names to link
    #[serde(default)]
    pub bin: Vec<String>,
    /// Post-install hints
    #[serde(default)]
    pub hints: String,
}

/// Package index (binary format)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PackageIndex {
    /// Index format version
    pub version: u32,
    /// Unix timestamp of last update
    pub updated_at: i64,
    /// Package entries
    pub packages: Vec<IndexEntry>,
}

impl PackageIndex {
    /// Create a new empty index
    pub fn new() -> Self {
        Self {
            version: 1,
            updated_at: 0,
            packages: Vec::new(),
        }
    }

    /// Load index from a zstd-compressed postcard file
    pub fn load(path: &Path) -> Result<Self, IndexError> {
        let compressed = fs::read(path)?;
        let decompressed = zstd::decode_all(compressed.as_slice())?;
        Ok(postcard::from_bytes(&decompressed)?)
    }

    /// Save index to a zstd-compressed postcard file
    pub fn save(&self, path: &Path) -> Result<(), IndexError> {
        let buf = postcard::to_allocvec(self)?;
        let file = fs::File::create(path)?;
        let mut encoder = zstd::stream::Encoder::new(file, 3)?;
        encoder.write_all(&buf)?;
        encoder.finish()?;
        Ok(())
    }

    /// Serialize to bytes (for network transfer)
    pub fn to_bytes(&self) -> Result<Vec<u8>, IndexError> {
        Ok(postcard::to_allocvec(self)?)
    }

    /// Deserialize from bytes
    pub fn from_bytes(data: &[u8]) -> Result<Self, IndexError> {
        Ok(postcard::from_bytes(data)?)
    }

    /// Add or update a package entry
    pub fn upsert(&mut self, entry: IndexEntry) {
        if let Some(existing) = self.packages.iter_mut().find(|e| e.name == entry.name) {
            *existing = entry;
        } else {
            self.packages.push(entry);
        }
    }

    /// Find a package by name
    pub fn find(&self, name: &str) -> Option<&IndexEntry> {
        self.packages.iter().find(|e| e.name == name)
    }

    /// Search packages by prefix
    pub fn search(&self, query: &str) -> Vec<&IndexEntry> {
        let query_lower = query.to_lowercase();
        self.packages
            .iter()
            .filter(|e| e.name.to_lowercase().contains(&query_lower))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_roundtrip() {
        let mut index = PackageIndex::new();
        index.upsert(IndexEntry {
            name: "neovim".to_string(),
            version: "0.10.0".to_string(),
            description: "Vim-fork focused on extensibility".to_string(),
            bottles: vec![
                IndexBottle {
                    arch: "aarch64-apple-darwin".to_string(),
                    url: "https://example.com/nvim.tar.zst".to_string(),
                    blake3: "abc123".to_string(),
                },
            ],
            deps: vec!["libuv".to_string()],
            bin: vec!["nvim".to_string()],
        });

        let bytes = index.to_bytes().unwrap();
        let restored = PackageIndex::from_bytes(&bytes).unwrap();

        assert_eq!(restored.packages.len(), 1);
        assert_eq!(restored.packages[0].name, "neovim");
    }

    #[test]
    fn test_search() {
        let mut index = PackageIndex::new();
        index.upsert(IndexEntry {
            name: "neovim".to_string(),
            version: "0.10.0".to_string(),
            description: String::new(),
            bottles: vec![],
            deps: vec![],
            bin: vec![],
        });
        index.upsert(IndexEntry {
            name: "vim".to_string(),
            version: "9.0".to_string(),
            description: String::new(),
            bottles: vec![],
            deps: vec![],
            bin: vec![],
        });

        let results = index.search("vim");
        assert_eq!(results.len(), 2); // neovim and vim
    }

    #[test]
    fn test_file_persistence() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("index.bin");

        let mut index = PackageIndex::new();
        index.updated_at = 1234567890;
        index.upsert(IndexEntry {
            name: "ripgrep".to_string(),
            version: "14.0.0".to_string(),
            description: "Fast grep".to_string(),
            bottles: vec![IndexBottle {
                arch: "aarch64-apple-darwin".to_string(),
                url: "https://example.com/rg".to_string(),
                blake3: "rg123".to_string(),
            }],
            deps: vec![],
            bin: vec![],
        });

        index.save(&path).unwrap();
        let loaded = PackageIndex::load(&path).unwrap();

        assert_eq!(loaded.updated_at, 1234567890);
        assert_eq!(loaded.packages[0].name, "ripgrep");
    }
}
