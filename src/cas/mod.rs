//! Content-Addressable Store (CAS)
//!
//! Stores files by their BLAKE3 hash, enabling deduplication and instant installs.

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use blake3::Hasher;
use thiserror::Error;

use crate::cas_path;

#[derive(Error, Debug)]
pub enum CasError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("File not found in CAS: {0}")]
    NotFound(String),
}

/// Content-Addressable Store
pub struct Cas {
    root: PathBuf,
}

impl Cas {
    /// Create a new CAS instance
    pub fn new() -> io::Result<Self> {
        let root = cas_path();
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    /// Create CAS at a custom path (for testing)
    pub fn with_root(root: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    /// Store bytes in the CAS, returning the BLAKE3 hash
    pub fn store_bytes(&self, data: &[u8]) -> io::Result<String> {
        let hash = blake3::hash(data).to_hex().to_string();
        let blob_path = self.blob_path(&hash);

        // Skip if already exists (content-addressed = immutable)
        if !blob_path.exists() {
            // Use 2-char prefix subdirectory for filesystem efficiency
            if let Some(parent) = blob_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&blob_path, data)?;
        }

        Ok(hash)
    }

    /// Store a file in the CAS, returning the BLAKE3 hash
    pub fn store_file(&self, path: &Path) -> Result<String, CasError> {
        let mut file = File::open(path)?;
        let mut hasher = Hasher::new();
        let mut buffer = [0u8; 65536]; // 64KB chunks

        loop {
            let bytes_read = file.read(&mut buffer)?;
            if bytes_read == 0 {
                break;
            }
            hasher.update(&buffer[..bytes_read]);
        }

        let hash = hasher.finalize().to_hex().to_string();
        let blob_path = self.blob_path(&hash);

        if !blob_path.exists() {
            if let Some(parent) = blob_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(path, &blob_path)?;
        }

        Ok(hash)
    }

    /// Retrieve bytes from the CAS by hash
    pub fn get(&self, hash: &str) -> Result<Vec<u8>, CasError> {
        let blob_path = self.blob_path(hash);
        if !blob_path.exists() {
            return Err(CasError::NotFound(hash.to_string()));
        }
        Ok(fs::read(&blob_path)?)
    }

    /// Check if a hash exists in the CAS
    pub fn contains(&self, hash: &str) -> bool {
        self.blob_path(hash).exists()
    }

    /// Link a CAS blob to a target path using hardlink (falls back to copy)
    pub fn link_to(&self, hash: &str, target: &Path) -> Result<(), CasError> {
        let blob_path = self.blob_path(hash);
        if !blob_path.exists() {
            return Err(CasError::NotFound(hash.to_string()));
        }

        // Ensure parent directory exists
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }

        // Try hardlink first (instant, no disk space)
        // On APFS, could use clonefile() for reflinks
        match fs::hard_link(&blob_path, target) {
            Ok(()) => Ok(()),
            Err(_) => {
                // Fall back to copy if hardlink fails (cross-filesystem)
                fs::copy(&blob_path, target)?;
                Ok(())
            }
        }
    }

    /// Get the path for a blob given its hash
    /// Uses 2-char prefix: ab/abcdef123...
    pub fn blob_path(&self, hash: &str) -> PathBuf {
        let prefix = &hash[..2.min(hash.len())];
        self.root.join(prefix).join(hash)
    }
}

impl Default for Cas {
    fn default() -> Self {
        Self::new().expect("Failed to create CAS")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_store_and_retrieve() {
        let dir = tempdir().unwrap();
        let cas = Cas::with_root(dir.path().to_path_buf()).unwrap();

        let data = b"hello, distill!";
        let hash = cas.store_bytes(data).unwrap();

        assert!(cas.contains(&hash));
        assert_eq!(cas.get(&hash).unwrap(), data);
    }

    #[test]
    fn test_deduplication() {
        let dir = tempdir().unwrap();
        let cas = Cas::with_root(dir.path().to_path_buf()).unwrap();

        let data = b"duplicate content";
        let hash1 = cas.store_bytes(data).unwrap();
        let hash2 = cas.store_bytes(data).unwrap();

        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_link_to() {
        let dir = tempdir().unwrap();
        let cas = Cas::with_root(dir.path().join("cas")).unwrap();

        let data = b"linkable content";
        let hash = cas.store_bytes(data).unwrap();

        let target = dir.path().join("bin").join("myfile");
        cas.link_to(&hash, &target).unwrap();

        assert!(target.exists());
        assert_eq!(fs::read(&target).unwrap(), data);
    }
}
