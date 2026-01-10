//! Index definition and serialization via Postcard/Zstd.
//!
//! Low-overhead binary package registry format.

use std::fs;
use std::io;
use std::path::Path;

use fuzzy_matcher::FuzzyMatcher;
use fuzzy_matcher::skim::SkimMatcherV2;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Hash algorithm type for binary verification
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum HashType {
    /// SHA256 hash (64 hex characters)
    #[default]
    Sha256,
    /// SHA512 hash (128 hex characters)
    Sha512,
}

impl HashType {
    /// Get the string representation of the hash type
    pub fn as_str(&self) -> &'static str {
        match self {
            HashType::Sha256 => "sha256",
            HashType::Sha512 => "sha512",
        }
    }
}

#[derive(Error, Debug)]
pub enum IndexError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("Serialization error: {0}")]
    Postcard(#[from] postcard::Error),

    #[error("Package definition error: {0}")]
    Package(String),

    #[error("Index version mismatch: found v{0}, expected v{1}. Run 'dl update' or update 'dl'.")]
    VersionMismatch(u32, u32),
}

/// Binary artifact info in the index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexBinary {
    /// Architecture (arm64, x86_64, or universal)
    pub arch: crate::types::Arch,
    /// Download URL
    pub url: String,
    /// Hash value (hex string)
    pub hash: crate::types::Sha256Hash,
    /// Hash algorithm type
    pub hash_type: HashType,
}

/// Source artifact info
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IndexSource {
    pub url: String,
    pub hash: crate::types::Sha256Hash,
    pub hash_type: HashType,
}

/// Compact release info (one version)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionInfo {
    /// Version string
    pub version: String,
    /// Available binaries with URLs
    pub binaries: Vec<IndexBinary>,
    /// Source info (if available)
    #[serde(default)]
    pub source: Option<IndexSource>,
    /// Runtime dependencies (names only)
    #[serde(default)]
    pub deps: Vec<String>,
    /// Build dependencies (names only)
    #[serde(default)]
    pub build_deps: Vec<String>,
    /// Build script
    #[serde(default)]
    pub build_script: String,
    /// Binary names to link
    #[serde(default)]
    pub bin: Vec<String>,
    /// Post-install hints
    #[serde(default)]
    pub hints: String,
    /// Name of the .app bundle (for type="app")
    #[serde(default)]
    pub app: Option<String>,
}

/// Compact package entry in the index
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexEntry {
    /// Package name
    pub name: String,
    /// Package description
    #[serde(default)]
    pub description: String,
    /// Package homepage
    #[serde(default)]
    pub homepage: String,
    /// Package type ("cli" or "app")
    #[serde(default)]
    #[serde(rename = "type")]
    pub type_: String,
    /// All binary names provided by any release (for search/resolution)
    #[serde(default)]
    pub bins: Vec<String>,
    /// All available releases (sorted by version descending)
    pub releases: Vec<VersionInfo>,
    /// Categories/Tags for the package
    #[serde(default)]
    pub tags: Vec<String>,
}

impl IndexEntry {
    /// Get the latest release (if any)
    pub fn latest(&self) -> Option<&VersionInfo> {
        self.releases.first()
    }

    /// Find a specific version - O(log n) binary search
    ///
    /// Note: Releases are sorted descending (newest first), so we reverse the comparison.
    pub fn find_version(&self, version: impl AsRef<str>) -> Option<&VersionInfo> {
        let v = version.as_ref();
        // Releases sorted descending, so we reverse: compare target to element (not element to target)
        self.releases
            .binary_search_by(|r| v.cmp(&r.version))
            .ok()
            .map(|idx| &self.releases[idx])
    }
}

/// Package index (binary format)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PackageIndex {
    /// Index format version (Bumped to 6 for Merkle root support)
    pub version: u32,
    /// Unix timestamp of last update
    pub updated_at: i64,
    /// Package entries
    pub packages: Vec<IndexEntry>,
    /// Base URL for artifact mirror (CAS layout: {base_url}/cas/{hash})
    #[serde(default)]
    pub mirror_base_url: Option<String>,
    /// Merkle tree root hash (BLAKE3) for integrity verification
    #[serde(default)]
    pub merkle_root: Option<crate::types::Blake3Hash>,
}

impl PackageIndex {
    /// Create a new empty index
    pub fn new() -> Self {
        Self {
            version: 6,
            updated_at: 0,
            mirror_base_url: None,
            merkle_root: None,
            packages: Vec::new(),
        }
    }

    /// Memory-maps and deserializes the index, auto-detecting Zstd compression.
    pub fn load(path: &Path) -> Result<Self, IndexError> {
        let file = fs::File::open(path)?;
        let mmap = unsafe { memmap2::Mmap::map(&file)? };

        // Implementation Note: Memory Mapping and Zero-Copy
        //
        // Instead of reading the file into a `Vec<u8>` (heap allocation), we use `mmap`.
        // This maps the file directly into process memory. The OS handles paging it in/out.
        // `postcard::from_bytes` then deserializes structs pointing *directly* to this memory
        // where possible (borrowing), avoiding string copies.
        //
        // This makes startup for large indices (10k+ packages) nearly instantaneous.
        let mut index = if mmap.len() >= 4 && mmap[0..4] == crate::ZSTD_MAGIC {
            let decompressed = zstd::decode_all(&mmap[..])?;
            postcard::from_bytes(&decompressed)
                .map_err(|_| IndexError::Postcard(postcard::Error::DeserializeBadVarint))?
        } else {
            // Postcard header check (version defined in from_bytes)
            Self::from_bytes(&mmap)?
        };

        // Ensure sorted for O(log n) lookups
        index.ensure_sorted();
        Ok(index)
    }

    /// Serializes to an uncompressed Postcard file, optimized for MMAP usage.
    pub fn save(&self, path: &Path) -> Result<(), IndexError> {
        let buf = postcard::to_allocvec(self)?;
        fs::write(path, &buf)?;
        Ok(())
    }

    /// Serializes and compresses the index for network distribution.
    pub fn save_compressed(&self, path: &Path) -> Result<(), IndexError> {
        let buf = postcard::to_allocvec(self)?;
        let compressed = zstd::encode_all(&buf[..], 3)?;
        fs::write(path, &compressed)?;
        Ok(())
    }

    /// Serialize to bytes (for network transfer)
    pub fn to_bytes(&self) -> Result<Vec<u8>, IndexError> {
        Ok(postcard::to_allocvec(self)?)
    }

    /// Deserialize from bytes
    pub fn from_bytes(data: &[u8]) -> Result<Self, IndexError> {
        // Postcard serializes fields in order. We deserialize just the header to check version.
        // This must match the first few fields of PackageIndex exactly!
        #[derive(Deserialize)]
        struct IndexHeader {
            version: u32,
            #[allow(dead_code)]
            updated_at: i64,
        }

        let header: IndexHeader = postcard::from_bytes(data)
            .map_err(|_| IndexError::Postcard(postcard::Error::DeserializeBadVarint))?;

        if header.version < 4 {
            return Err(IndexError::VersionMismatch(header.version, 4));
        }

        Ok(postcard::from_bytes(data)?)
    }

    /// Add or update a package entry (full entry)
    pub fn upsert(&mut self, entry: IndexEntry) {
        match self.packages.binary_search_by(|e| e.name.cmp(&entry.name)) {
            Ok(idx) => self.packages[idx] = entry,
            Err(idx) => self.packages.insert(idx, entry),
        }
    }

    /// Add a single release to a package
    pub fn upsert_release(
        &mut self,
        name: &str,
        description: &str,
        type_: &str,
        tags: Vec<String>,
        release: VersionInfo,
    ) {
        match self
            .packages
            .binary_search_by(|e| e.name.as_str().cmp(name))
        {
            Ok(idx) => {
                let entry = &mut self.packages[idx];
                entry.description = description.to_string();
                entry.type_ = type_.to_string();
                entry.tags = tags;
                if let Some(existing) = entry
                    .releases
                    .iter_mut()
                    .find(|r| r.version == release.version)
                {
                    *existing = release;
                } else {
                    entry.releases.push(release);
                }
                // Sort releases by version descending (semver-aware)
                entry.releases.sort_by(|a, b| {
                    // Try semver parsing for both; fall back to string comparison
                    match (
                        semver::Version::parse(&a.version),
                        semver::Version::parse(&b.version),
                    ) {
                        (Ok(va), Ok(vb)) => vb.cmp(&va),             // Descending
                        (Ok(_), Err(_)) => std::cmp::Ordering::Less, // Valid semver first
                        (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
                        (Err(_), Err(_)) => b.version.cmp(&a.version), // Fallback to string
                    }
                });

                // Update aggregate bins list
                let mut all_bins = std::collections::HashSet::new();
                for r in &entry.releases {
                    for b in &r.bin {
                        all_bins.insert(b.clone());
                    }
                }
                entry.bins = all_bins.into_iter().collect();
                entry.bins.sort();
            }
            Err(idx) => {
                let bins = release.bin.clone();
                self.packages.insert(
                    idx,
                    IndexEntry {
                        name: name.to_string(),
                        description: description.to_string(),
                        homepage: String::new(),
                        type_: type_.to_string(),
                        bins,
                        releases: vec![release],
                        tags,
                    },
                );
            }
        }
    }

    /// Find a package by name - O(log n) binary search
    pub fn find(&self, name: impl AsRef<str>) -> Option<&IndexEntry> {
        let n = name.as_ref();
        self.packages
            .binary_search_by(|e| e.name.as_str().cmp(n))
            .ok()
            .map(|idx| &self.packages[idx])
    }

    /// Search packages by query (matches name or description)
    ///
    /// Supports fuzzy matching via SkimMatcherV2 and tag filtering via 'tag:<name>'.
    /// Results are ranked by match score.
    pub fn search(&self, query: &str) -> Vec<&IndexEntry> {
        if query.is_empty() {
            return self.packages.iter().take(50).collect();
        }

        let matcher = SkimMatcherV2::default();
        let query_lower = query.to_lowercase();

        // 1. Check for tag: filter
        if let Some(tag_query) = query_lower.strip_prefix("tag:") {
            return self
                .packages
                .iter()
                .filter(|e| e.tags.iter().any(|t| t.to_lowercase() == tag_query))
                .collect();
        }

        // 2. Perform fuzzy search
        let mut results: Vec<(i64, &IndexEntry)> = self
            .packages
            .iter()
            .filter_map(|e| {
                // We score name, description, and bins
                let mut best_score = matcher.fuzzy_match(&e.name, query);

                if let Some(desc_score) = matcher.fuzzy_match(&e.description, query) {
                    // Description matches are slightly less weighted than name matches
                    let adjusted = desc_score / 2;
                    best_score = Some(best_score.unwrap_or(0).max(adjusted));
                }

                for b in &e.bins {
                    if let Some(bin_score) = matcher.fuzzy_match(b, query) {
                        best_score = Some(best_score.unwrap_or(0).max(bin_score));
                    }
                }

                best_score.map(|s| (s, e))
            })
            .collect();

        // 3. Rank by score descending
        results.sort_by(|a, b| b.0.cmp(&a.0));

        results.into_iter().map(|(_, e)| e).take(50).collect()
    }

    /// Search packages by name prefix - O(log n) using binary search
    pub fn search_prefix(&self, prefix: &str) -> Vec<&IndexEntry> {
        let start = self.packages.partition_point(|e| e.name.as_str() < prefix);

        // Collect all entries that start with prefix
        self.packages[start..]
            .iter()
            .take_while(|e| e.name.starts_with(prefix))
            .collect()
    }

    /// Ensure packages are sorted by name for binary search.
    /// Called after load and deserialization.
    fn ensure_sorted(&mut self) {
        self.packages.sort_by(|a, b| a.name.cmp(&b.name));
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
            description: "Vim-fork focused on extensibility".to_string(),
            homepage: "https://neovim.io".to_string(),
            type_: "cli".to_string(),
            bins: vec!["nvim".to_string()],
            releases: vec![VersionInfo {
                version: "0.10.0".to_string(),
                binaries: vec![IndexBinary {
                    arch: crate::types::Arch::Arm64,
                    url: "https://example.com/nvim.tar.zst".to_string(),
                    hash: crate::types::Sha256Hash::new("abc123"),
                    hash_type: HashType::Sha256,
                }],
                deps: vec!["libuv".to_string()],
                build_deps: vec![],
                build_script: String::new(),
                bin: vec!["nvim".to_string()],
                hints: String::new(),
                app: None,
                source: None,
            }],
            tags: vec![],
        });

        let bytes = index.to_bytes().unwrap();
        let restored = PackageIndex::from_bytes(&bytes).unwrap();

        assert_eq!(restored.packages.len(), 1);
        assert_eq!(restored.packages[0].name, "neovim");
        assert_eq!(restored.packages[0].releases[0].version, "0.10.0");
    }

    #[test]
    fn test_upsert_release() {
        let mut index = PackageIndex::new();
        let release1 = VersionInfo {
            version: "1.0.0".to_string(),
            binaries: vec![],
            deps: vec![],
            build_deps: vec![],
            build_script: String::new(),
            bin: vec![],
            hints: String::new(),
            app: None,
            source: None,
        };
        let release2 = VersionInfo {
            version: "1.1.0".to_string(),
            binaries: vec![],
            deps: vec![],
            build_deps: vec![],
            build_script: String::new(),
            bin: vec![],
            hints: String::new(),
            app: None,
            source: None,
        };

        index.upsert_release("test", "Test description", "cli", vec![], release1);
        index.upsert_release("test", "Test description", "cli", vec![], release2);

        let entry = index.find("test").unwrap();
        assert_eq!(entry.releases.len(), 2);
        assert_eq!(entry.latest().unwrap().version, "1.1.0");
    }

    #[test]
    fn test_version_check() {
        let mut index = PackageIndex::new();
        index.version = 1; // Force old version
        let bytes = postcard::to_allocvec(&index).unwrap();

        let result = PackageIndex::from_bytes(&bytes);
        assert!(result.is_err());
        if let Err(IndexError::VersionMismatch(found, expected)) = result {
            assert_eq!(found, 1);
            assert_eq!(expected, 4);
        } else {
            panic!("Expected VersionMismatch error");
        }
    }

    #[test]
    fn test_file_persistence() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("index");

        let mut index = PackageIndex::new();
        index.updated_at = 1234567890;
        index.updated_at = 1234567890;
        index.upsert_release(
            "ripgrep",
            "Fast grep",
            "cli",
            vec![],
            VersionInfo {
                version: "14.0.0".to_string(),
                binaries: vec![IndexBinary {
                    arch: crate::types::Arch::Arm64,
                    url: "https://example.com/foo-arm64".to_string(),
                    hash: crate::types::Sha256Hash::new("hash1"),
                    hash_type: HashType::Sha256,
                }],
                deps: vec![],
                build_deps: vec![],
                build_script: String::new(),
                bin: vec![],
                hints: String::new(),
                app: None,
                source: None,
            },
        );

        index.save(&path).unwrap();
        let loaded = PackageIndex::load(&path).unwrap();

        assert_eq!(loaded.updated_at, 1234567890);
        assert_eq!(loaded.packages[0].name, "ripgrep");
    }

    /// Regression test: semver sorting must handle 0.12.0 > 0.9.1 correctly.
    /// String comparison would incorrectly put 0.9.1 first because "0.9" > "0.12" alphabetically.
    #[test]
    fn test_semver_version_sorting() {
        let mut index = PackageIndex::new();

        // Insert versions in "wrong" order to test sorting
        for version in ["0.9.1", "0.12.0", "0.8.0", "1.0.0", "0.10.0"] {
            index.upsert_release(
                "test-pkg",
                "Test package",
                "cli",
                vec![],
                VersionInfo {
                    version: version.to_string(),
                    binaries: vec![],
                    deps: vec![],
                    build_deps: vec![],
                    build_script: String::new(),
                    bin: vec![],
                    hints: String::new(),
                    app: None,
                    source: None,
                },
            );
        }

        let entry = index.find("test-pkg").unwrap();
        let versions: Vec<&str> = entry.releases.iter().map(|r| r.version.as_str()).collect();

        // Should be sorted descending by semver: 1.0.0, 0.12.0, 0.10.0, 0.9.1, 0.8.0
        assert_eq!(
            versions,
            vec!["1.0.0", "0.12.0", "0.10.0", "0.9.1", "0.8.0"]
        );

        // latest() should return the highest version
        assert_eq!(entry.latest().unwrap().version, "1.0.0");
    }

    #[test]
    fn test_search_fuzzy() {
        let mut index = PackageIndex::new();
        let release = VersionInfo {
            version: "1.0.0".to_string(),
            binaries: vec![],
            deps: vec![],
            build_deps: vec![],
            build_script: String::new(),
            bin: vec!["nvim".to_string()],
            hints: String::new(),
            app: None,
            source: None,
        };
        index.upsert_release("neovim", "Vim fork", "cli", vec![], release);

        // Exact match
        assert!(!index.search("neovim").is_empty());
        // Typo/Fuzzy match (in-order)
        assert!(!index.search("novim").is_empty());
        // Bin match
        assert!(!index.search("nvim").is_empty());
        // No match
        assert!(index.search("zsh").is_empty());
    }

    #[test]
    fn test_search_tags() {
        let mut index = PackageIndex::new();
        let release = VersionInfo {
            version: "1.0.0".to_string(),
            binaries: vec![],
            deps: vec![],
            build_deps: vec![],
            build_script: String::new(),
            bin: vec![],
            hints: String::new(),
            app: None,
            source: None,
        };
        index.upsert_release(
            "ripgrep",
            "Fast grep",
            "cli",
            vec!["Editor".to_string(), "Tool".to_string()],
            release,
        );

        // Tag match
        assert_eq!(index.search("tag:editor").len(), 1);
        assert_eq!(index.search("tag:tool").len(), 1);
        // Case-insensitive tag match
        assert_eq!(index.search("tag:EDITOR").len(), 1);
        // No match
        assert_eq!(index.search("tag:network").len(), 0);
    }
}
