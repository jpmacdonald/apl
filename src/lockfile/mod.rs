//! Lockfile module for reproducible installs
//!
//! The lockfile (`dl.lock`) pins exact versions and hashes.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum LockfileError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Parse error: {0}")]
    Parse(#[from] toml::de::Error),

    #[error("Serialize error: {0}")]
    Serialize(#[from] toml::ser::Error),
}

/// A locked package entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockedPackage {
    pub name: String,
    pub version: String,
    pub blake3: String,
    pub url: String,
    #[serde(default)]
    pub arch: String,
}

/// The lockfile structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lockfile {
    /// Lockfile format version
    pub version: u32,
    /// When the lockfile was generated
    pub generated_at: String,
    /// Locked packages
    #[serde(default)]
    pub packages: Vec<LockedPackage>,
}

impl Lockfile {
    /// Create a new empty lockfile
    pub fn new() -> Self {
        Self {
            version: 1,
            generated_at: now_iso8601(),
            packages: Vec::new(),
        }
    }

    /// Load lockfile from path
    pub fn load(path: &Path) -> Result<Self, LockfileError> {
        let content = fs::read_to_string(path)?;
        Ok(toml::from_str(&content)?)
    }

    /// Save lockfile to path
    pub fn save(&self, path: &Path) -> Result<(), LockfileError> {
        let content = toml::to_string_pretty(self)?;
        fs::write(path, content)?;
        Ok(())
    }

    /// Add or update a package in the lockfile
    pub fn add_package(&mut self, pkg: LockedPackage) {
        // Remove existing entry for this package
        self.packages.retain(|p| p.name != pkg.name);
        self.packages.push(pkg);
        // Sort by name for consistent ordering
        self.packages.sort_by(|a, b| a.name.cmp(&b.name));
        self.generated_at = now_iso8601();
    }

    /// Find a package by name
    pub fn find(&self, name: &str) -> Option<&LockedPackage> {
        self.packages.iter().find(|p| p.name == name)
    }

    /// Check if lockfile exists at default path
    pub fn exists_default() -> bool {
        Path::new("dl.lock").exists()
    }

    /// Load from default path (dl.lock in current directory)
    pub fn load_default() -> Result<Self, LockfileError> {
        Self::load(Path::new("dl.lock"))
    }

    /// Save to default path
    pub fn save_default(&self) -> Result<(), LockfileError> {
        self.save(Path::new("dl.lock"))
    }
}

impl Default for Lockfile {
    fn default() -> Self {
        Self::new()
    }
}

/// Get current time in ISO 8601 format
fn now_iso8601() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap();
    
    // Simple ISO 8601 without external crate
    let secs = duration.as_secs();
    let days = secs / 86400;
    let remaining = secs % 86400;
    let hours = remaining / 3600;
    let minutes = (remaining % 3600) / 60;
    let seconds = remaining % 60;
    
    // Approximate date calculation (good enough for display)
    let years = 1970 + (days / 365);
    let day_of_year = days % 365;
    let month = (day_of_year / 30) + 1;
    let day = (day_of_year % 30) + 1;
    
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        years, month.min(12), day.min(31), hours, minutes, seconds
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_lockfile_roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("dl.lock");

        let mut lockfile = Lockfile::new();
        lockfile.add_package(LockedPackage {
            name: "jq".to_string(),
            version: "1.7.1".to_string(),
            blake3: "abc123".to_string(),
            url: "https://example.com/jq".to_string(),
            arch: "arm64".to_string(),
        });

        lockfile.save(&path).unwrap();
        let loaded = Lockfile::load(&path).unwrap();

        assert_eq!(loaded.packages.len(), 1);
        assert_eq!(loaded.packages[0].name, "jq");
    }

    #[test]
    fn test_find_package() {
        let mut lockfile = Lockfile::new();
        lockfile.add_package(LockedPackage {
            name: "ripgrep".to_string(),
            version: "14.0.0".to_string(),
            blake3: "def456".to_string(),
            url: "https://example.com/rg".to_string(),
            arch: "arm64".to_string(),
        });

        assert!(lockfile.find("ripgrep").is_some());
        assert!(lockfile.find("nonexistent").is_none());
    }
}
