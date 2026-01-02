use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tokio::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub project: ProjectObj,
    pub dependencies: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectObj {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lockfile {
    pub package: Vec<LockPackage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockPackage {
    pub name: String,
    pub version: String,
    pub url: String,
    pub blake3: String,
    pub timestamp: Option<i64>,
}

impl Manifest {
    pub async fn load(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path)
            .await
            .context("Failed to read apl.toml")?;

        let manifest: Manifest = toml::from_str(&content).context("Failed to parse apl.toml")?;

        Ok(manifest)
    }
}

impl Lockfile {
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

    pub async fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self)?;

        // Atomic write: write to temp file, then rename
        let temp_path = path.with_extension("lock.tmp");
        fs::write(&temp_path, &content).await?;
        fs::rename(&temp_path, path).await?;

        Ok(())
    }
}
