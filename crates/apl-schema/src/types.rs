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

/// Errors that can occur when validating an [`Artifact`].
#[derive(thiserror::Error, Debug)]
pub enum ArtifactError {
    /// The SHA-256 hash string is not exactly 64 characters long.
    #[error("Invalid SHA256 length: expected 64 chars, got {0}")]
    InvalidSha256Length(usize),

    /// A required field (name, version, or URL) is empty.
    #[error("Empty field: {0}")]
    EmptyField(String),

    /// The download URL is malformed or uses an unsupported scheme.
    #[error("Invalid URL: {0}")]
    InvalidUrl(String),
}

impl Artifact {
    /// Validates the artifact's integrity by checking all required fields.
    ///
    /// # Errors
    ///
    /// Returns [`ArtifactError::EmptyField`] if `name`, `version`, or `url` is empty,
    /// [`ArtifactError::InvalidUrl`] if the URL does not start with `http`,
    /// or [`ArtifactError::InvalidSha256Length`] if the hash is not 64 characters.
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

/// Whether a package provides a command-line tool or a GUI application.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PackageType {
    /// Command-line interface tool (default); installed by symlinking binaries.
    #[default]
    Cli,
    /// GUI application; installed by copying the `.app` bundle.
    App,
}

/// Archive or binary format of a downloadable artifact.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactFormat {
    /// Gzip-compressed tar archive (`.tar.gz` / `.tgz`).
    #[serde(rename = "tar.gz")]
    TarGz,
    /// Zstandard-compressed tar archive (`.tar.zst`).
    #[serde(rename = "tar.zst")]
    TarZst,
    /// Uncompressed tar archive (`.tar`).
    Tar,
    /// Zip archive (`.zip`).
    Zip,
    /// macOS disk image (`.dmg`).
    Dmg,
    /// macOS installer package (`.pkg`).
    Pkg,
    /// Standalone executable with no archive wrapper.
    Binary,
}

/// How a package should be installed after extraction.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum InstallStrategy {
    /// Create symlinks for the declared binaries into the APL bin directory (default).
    #[default]
    Link,
    /// Copy the `.app` bundle into the APL applications directory.
    App,
    /// Run the macOS `.pkg` installer.
    Pkg,
    /// Execute a custom post-install shell script.
    Script,
}

/// Declarative configuration for a Port, parsed from `port.toml`.
///
/// Each variant corresponds to a discovery/download strategy that the
/// build engine uses to resolve versions and fetch artifacts.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "strategy", rename_all = "lowercase")]
pub enum PortConfig {
    /// `HashiCorp` releases API strategy (Terraform, Vault, Consul, etc.).
    #[serde(rename = "hashicorp")]
    HashiCorp {
        /// `HashiCorp` product slug used in the releases API (e.g. `terraform`).
        product: String,
    },

    /// Official Go toolchain downloads from `go.dev`.
    #[serde(rename = "golang")]
    Golang,

    /// Official Node.js downloads from `nodejs.org`.
    #[serde(rename = "node")]
    Node,

    /// GitHub Releases strategy, discovering versions from release tags.
    #[serde(rename = "github")]
    GitHub {
        /// GitHub repository owner (user or organization).
        owner: String,
        /// GitHub repository name.
        repo: String,
    },

    /// Fallback strategy for packages that need bespoke discovery logic.
    #[serde(rename = "custom")]
    Custom,

    /// AWS CLI downloads from the official Amazon distribution.
    #[serde(rename = "aws")]
    Aws,

    /// `CPython` releases from `python.org`.
    #[serde(rename = "python")]
    Python,

    /// Ruby releases from the official Ruby distribution.
    #[serde(rename = "ruby")]
    Ruby,

    /// Build-from-source strategy with a source tarball and build script.
    #[serde(rename = "build")]
    Build {
        /// URL template for the source tarball.
        source_url: String,
        /// Build specification (tag pattern, script, dependencies).
        #[serde(default)]
        #[serde(flatten)]
        spec: BuildSpec,
    },
}

/// Specification for building a package from source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuildSpec {
    /// Regex pattern to extract version numbers from upstream Git tags.
    pub tag_pattern: String,
    /// Optional regex or glob to filter versions discovered from the source (e.g. "3.12.*")
    pub version_pattern: Option<String>,
    /// Build script (runs in sysroot)
    #[serde(default)]
    pub script: String,
    /// Build-time dependencies (e.g. cmake, ninja)
    #[serde(default)]
    pub dependencies: Vec<String>,
}

/// Top-level structure for `port.toml`.
#[derive(Debug, Serialize, Deserialize)]
pub struct PortManifest {
    /// The `[package]` table containing name, strategy, and optional URL.
    pub package: PackageMeta,
}

/// Metadata about a package as declared in the `[package]` section of `port.toml`.
#[derive(Debug, Serialize, Deserialize)]
pub struct PackageMeta {
    /// Package name as it appears in the registry (e.g. `terraform`).
    pub name: String,
    /// Flattened port configuration that determines the discovery strategy.
    #[serde(flatten)]
    pub config: PortConfig,

    /// Optional URL override if strategy needs a base URL
    pub url: Option<String>,
}

/// A normalized package name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PackageName(String);

impl PackageName {
    /// Create a new package name, normalizing the input to lowercase.
    pub fn new(name: &str) -> Self {
        Self(name.to_lowercase())
    }

    /// Return the normalized name as a string slice.
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
    /// Create a new version from the given string (stored as-is).
    pub fn new(v: &str) -> Self {
        Self(v.to_string())
    }

    /// Return the version string as a string slice.
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
