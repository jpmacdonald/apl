//! Manifest and lockfile parsing for APL projects.
//!
//! An APL manifest (`apl.toml`) declares a project's identity and its
//! dependencies.  The companion lockfile (`apl.lock`) records the exact
//! resolved versions and artifact URLs so that builds are reproducible.

use crate::types::{PackageName, Version};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tokio::fs;

/// Top-level project manifest parsed from an `apl.toml` file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// Project identity metadata.
    pub project: ProjectObj,
    /// Map of dependency names to version requirement strings.
    pub dependencies: HashMap<PackageName, String>,
}

/// The `[project]` section of an APL manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectObj {
    /// Human-readable name of the project.
    pub name: String,
}

/// A resolved lockfile containing pinned package versions and artifact URLs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lockfile {
    /// Ordered list of locked packages with their resolved metadata.
    pub package: Vec<LockPackage>,
}

/// A single entry in the lockfile representing one resolved package.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockPackage {
    /// The package name as it appears in the registry.
    pub name: PackageName,
    /// The exact resolved version.
    pub version: Version,
    /// Download URL for the package artifact.
    pub url: String,
    /// SHA-256 digest of the artifact for integrity verification.
    pub sha256: String,
    /// Unix timestamp recording when this entry was locked.
    pub timestamp: Option<i64>,
}

impl Manifest {
    /// Asynchronously load and parse a `Manifest` from the given file path.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or if its contents are not
    /// valid TOML conforming to the manifest schema.
    pub async fn load(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)
            .await
            .context("Failed to read apl.toml")?;

        let manifest: Manifest = toml::from_str(&content).context("Failed to parse apl.toml")?;

        Ok(manifest)
    }
}

impl Lockfile {
    /// Asynchronously load and parse a `Lockfile` from the given file path.
    ///
    /// If the file does not exist, an empty `Lockfile` is returned so that
    /// callers can treat the first resolution the same as subsequent ones.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be read or parsed.
    pub async fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Lockfile { package: vec![] });
        }

        let content = fs::read_to_string(path)
            .await
            .context("Failed to read apl.lock")?;

        let lock: Lockfile = toml::from_str(&content).context("Failed to parse apl.lock")?;

        Ok(lock)
    }

    /// Atomically persist this `Lockfile` to disk at the given path.
    ///
    /// The file is first written to a temporary location and then renamed so
    /// that readers never observe a partially written lockfile.
    ///
    /// # Errors
    ///
    /// Returns an error if serialization, file writing, or the atomic rename
    /// fails.
    pub async fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self)?;

        // Atomic write: write to temp file, then rename
        let temp_path = path.with_extension("lock.tmp");
        fs::write(&temp_path, &content).await?;
        fs::rename(&temp_path, path).await?;

        Ok(())
    }
}
