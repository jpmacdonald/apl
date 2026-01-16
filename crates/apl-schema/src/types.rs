use serde::{Deserialize, Serialize};
use std::borrow::Borrow;

/// Represents an artifact in the APL index (e.g., index.json).
/// This structure is shared between the Engine (producer) and Core (consumer).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Artifact {
    /// Name of the package (e.g., "terraform")
    pub name: String,

    /// Version string (e.g., "1.5.0")
    pub version: String,

    /// Architecture this artifact supports (e.g., "x86_64-apple-darwin")
    pub arch: String,

    /// Download URL (original vendor URL or R2 mirror)
    pub url: String,

    /// SHA256 Checksum
    pub sha256: String,
}

#[derive(thiserror::Error, Debug)]
pub enum ArtifactError {
    #[error("Invalid SHA256 length: expected 64 chars, got {0}")]
    InvalidSha256Length(usize),

    #[error("Empty field: {0}")]
    EmptyField(String),

    #[error("Invalid URL: {0}")]
    InvalidUrl(String),
}

impl Artifact {
    /// Validates the artifact's integrity.
    pub fn validate(&self) -> Result<(), ArtifactError> {
        if self.name.is_empty() {
            return Err(ArtifactError::EmptyField("name".to_string()));
        }
        if self.version.is_empty() {
            return Err(ArtifactError::EmptyField("version".to_string()));
        }
        if self.url.is_empty() {
            return Err(ArtifactError::EmptyField("url".to_string()));
        }
        if !self.url.starts_with("http") {
            return Err(ArtifactError::InvalidUrl(
                "Must start with http(s)".to_string(),
            ));
        }

        // Strict SHA256 validation
        if self.sha256.len() != 64 {
            return Err(ArtifactError::InvalidSha256Length(self.sha256.len()));
        }

        Ok(())
    }
}

/// Package type
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PackageType {
    #[default]
    Cli,
    App,
}

/// Artifact format
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactFormat {
    #[serde(rename = "tar.gz")]
    TarGz,
    #[serde(rename = "tar.zst")]
    TarZst,
    Tar,
    Zip,
    Dmg,
    Pkg,
    Binary,
}

/// Installation strategy
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum InstallStrategy {
    #[default]
    Link,
    App,
    Pkg,
    Script,
}

/// Declarative configuration for a Port, parsed from `port.toml`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "strategy", rename_all = "lowercase")]
pub enum PortConfig {
    /// Generic JSON feed strategy (e.g., HashiCorp, Go, Node)
    #[serde(rename = "hashicorp")]
    HashiCorp { product: String },

    #[serde(rename = "golang")]
    Golang,

    #[serde(rename = "node")]
    Node,

    #[serde(rename = "github")]
    GitHub { owner: String, repo: String },

    #[serde(rename = "custom")]
    Custom, // Fallback for complex logic

    #[serde(rename = "aws")]
    Aws,

    #[serde(rename = "python")]
    Python,

    #[serde(rename = "ruby")]
    Ruby,
}

/// Top-level structure for `port.toml`
#[derive(Debug, Serialize, Deserialize)]
pub struct PortManifest {
    pub package: PackageMeta,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PackageMeta {
    pub name: String,
    #[serde(flatten)]
    pub config: PortConfig,

    /// Optional URL override if strategy needs a base URL
    pub url: Option<String>,
}

/// A normalized package name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PackageName(String);

impl PackageName {
    pub fn new(name: &str) -> Self {
        Self(name.to_lowercase())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<std::ffi::OsStr> for PackageName {
    fn as_ref(&self) -> &std::ffi::OsStr {
        self.0.as_ref()
    }
}

impl AsRef<std::path::Path> for PackageName {
    fn as_ref(&self) -> &std::path::Path {
        std::path::Path::new(&self.0)
    }
}

impl std::fmt::Display for PackageName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::ops::Deref for PackageName {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<str> for PackageName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl PartialEq<str> for PackageName {
    fn eq(&self, other: &str) -> bool {
        self.0 == other.to_lowercase()
    }
}

impl PartialEq<&str> for PackageName {
    fn eq(&self, other: &&str) -> bool {
        self.0 == other.to_lowercase()
    }
}

impl PartialEq<String> for PackageName {
    fn eq(&self, other: &String) -> bool {
        self.0 == other.to_lowercase()
    }
}

impl Borrow<str> for PackageName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl From<&str> for PackageName {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for PackageName {
    fn from(s: String) -> Self {
        Self::new(&s)
    }
}

/// A semantic version string.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Version(String);

impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (
            semver::Version::parse(&self.0),
            semver::Version::parse(&other.0),
        ) {
            (Ok(a), Ok(b)) => a.cmp(&b),
            (Ok(_), Err(_)) => std::cmp::Ordering::Less,
            (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
            (Err(_), Err(_)) => self.0.cmp(&other.0),
        }
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Version {
    pub fn new(v: &str) -> Self {
        Self(v.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::ops::Deref for Version {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<str> for Version {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<&str> for Version {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for Version {
    fn from(s: String) -> Self {
        Self::new(&s)
    }
}

impl PartialEq<str> for Version {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for Version {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<String> for Version {
    fn eq(&self, other: &String) -> bool {
        self.0 == *other
    }
}

impl AsRef<std::path::Path> for Version {
    fn as_ref(&self) -> &std::path::Path {
        std::path::Path::new(&self.0)
    }
}
