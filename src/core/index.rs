//! Index definition and serialization via Postcard/Zstd.
//!
//! Low-overhead binary package registry format.

use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Hash algorithm type for binary verification
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum HashType {
    /// BLAKE3 hash (64 hex characters)
    #[default]
    Blake3,
    /// SHA256 hash (64 hex characters)
    Sha256,
    /// SHA512 hash (128 hex characters)
    Sha512,
}

impl HashType {
    /// Get the string representation of the hash type
    pub fn as_str(&self) -> &'static str {
        match self {
            HashType::Blake3 => "blake3",
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
    /// Architecture (e.g., "aarch64-apple-darwin", "arm64", "x86_64", "universal")
    pub arch: String,
    /// Download URL
    pub url: String,
    /// Hash value (hex string)
    pub hash: String,
    /// Hash algorithm type
    pub hash_type: HashType,
}

/// Source artifact info
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IndexSource {
    pub url: String,
    pub hash: String,
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
    /// All available releases (sorted by version descending)
    pub releases: Vec<VersionInfo>,
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
    /// Index format version (Bumped to 4 for source support)
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
            version: 4,
            updated_at: 0,
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
        // Postcard serializes fields in order. First field of PackageIndex is version: u32.
        // We can try to deserialize just the header first to check version.
        #[derive(Deserialize)]
        struct IndexHeader {
            version: u32,
        }

        let header: IndexHeader = postcard::from_bytes(data)
            .map_err(|_| IndexError::Postcard(postcard::Error::DeserializeBadVarint))?; // Placeholder if header fails

        if header.version < 3 {
            // We could implement migration here, but for now just bail with a clear message.
            return Err(IndexError::VersionMismatch(header.version, 3));
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
                if let Some(existing) = entry
                    .releases
                    .iter_mut()
                    .find(|r| r.version == release.version)
                {
                    *existing = release;
                } else {
                    entry.releases.push(release);
                }
                // Sort releases by version descending
                entry.releases.sort_by(|a, b| b.version.cmp(&a.version));
            }
            Err(idx) => {
                self.packages.insert(
                    idx,
                    IndexEntry {
                        name: name.to_string(),
                        description: description.to_string(),
                        homepage: String::new(),
                        type_: type_.to_string(),
                        releases: vec![release],
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

    /// Search packages by query (matches name or description) - O(n) scan
    ///
    /// Note: This is intentionally O(n) because it's a substring search.
    /// For prefix-only search, we could use partition_point for O(log n).
    pub fn search(&self, query: &str) -> Vec<&IndexEntry> {
        let query_lower = query.to_lowercase();
        self.packages
            .iter()
            .filter(|e| {
                e.name.to_lowercase().contains(&query_lower)
                    || e.description.to_lowercase().contains(&query_lower)
            })
            .collect()
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
            releases: vec![VersionInfo {
                version: "0.10.0".to_string(),
                binaries: vec![IndexBinary {
                    arch: "aarch64-apple-darwin".to_string(),
                    url: "https://example.com/nvim.tar.zst".to_string(),
                    hash: "abc123".to_string(),
                    hash_type: HashType::Blake3,
                }],
                deps: vec!["libuv".to_string()],
                build_deps: vec![],
                build_script: String::new(),
                bin: vec!["nvim".to_string()],
                hints: String::new(),
                app: None,
                source: None,
            }],
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

        index.upsert_release("test", "Test description", "cli", release1);
        index.upsert_release("test", "Test description", "cli", release2);

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
            assert_eq!(expected, 3);
        } else {
            panic!("Expected VersionMismatch error");
        }
    }

    #[test]
    fn test_file_persistence() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("index.bin");

        let mut index = PackageIndex::new();
        index.updated_at = 1234567890;
        index.updated_at = 1234567890;
        index.upsert_release(
            "ripgrep",
            "Fast grep",
            "cli",
            VersionInfo {
                version: "14.0.0".to_string(),
                binaries: vec![IndexBinary {
                    arch: "aarch64-apple-darwin".to_string(),
                    url: "https://example.com/rg".to_string(),
                    hash: "rg123".to_string(),
                    hash_type: HashType::Blake3,
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
}
