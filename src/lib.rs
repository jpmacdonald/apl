//! apl - A Package Layer
//!
//! Fast, minimal package manager for macOS CLI tools.
//!
//! # Overview
//!
//! APL provides a streamlined way to install, manage, and update command-line tools
//! on macOS. It uses a binary package index for fast lookups and supports both
//! pre-compiled binaries and building from source.
//!
//! # Architecture
//!
//! - **Typestate Pattern**: The installation flow uses `UnresolvedPackage` →
//!   `ResolvedPackage` → `PreparedPackage` to enforce correct ordering at compile time.
//! - **Actor Pattern**: Database access is serialized through `DbHandle` for thread safety.
//! - **Newtypes**: `PackageName`, `Version`, and `Blake3Hash` provide type-safe identifiers.
//!
//! # Directory Layout
//!
//! ```text
//! ~/.apl/
//! ├── bin/        # Symlinks to active binaries
//! ├── store/      # Package artifacts by name/version
//! ├── cache/      # Downloaded archives (by hash)
//! ├── index.bin   # Binary package index
//! └── state.db    # SQLite database
//! ```

pub mod core;
pub mod io;
pub mod ops;
pub mod registry;
pub mod store;
pub mod ui;

// Re-exports for convenience
pub use core::index;
pub use core::package;
pub use core::resolver;
pub use io::download as downloader;
pub use io::extract as extractor;
pub use store::DbHandle;
pub use store::db;

use dirs::home_dir;
use std::path::PathBuf;

/// Returns the primary configuration directory, or None if the user's home cannot be resolved.
pub fn try_apl_home() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".apl"))
}

/// Returns the canonical APL home directory (`~/.apl`).
///
/// # Panics
/// Panics if the home directory cannot be determined.
pub fn apl_home() -> PathBuf {
    try_apl_home().expect("Could not determine home directory")
}

/// SQLite database path: ~/.apl/state.db
pub fn db_path() -> PathBuf {
    apl_home().join("state.db")
}

/// Package store path: ~/.apl/store
pub fn store_path() -> PathBuf {
    apl_home().join("store")
}

/// Binary installation target: ~/.apl/bin
pub fn bin_path() -> PathBuf {
    apl_home().join("bin")
}

/// Cache path: ~/.apl/cache
pub fn cache_path() -> PathBuf {
    apl_home().join("cache")
}

/// Logs directory: ~/.apl/logs
pub fn log_dir() -> PathBuf {
    apl_home().join("logs")
}

/// Generate a build log path for a package
pub fn build_log_path(package: &str, version: &str) -> PathBuf {
    let timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    log_dir().join(format!("build-{package}-{version}-{timestamp}.log"))
}

/// Temp path: ~/.apl/tmp (guaranteed same volume as store)
pub fn tmp_path() -> PathBuf {
    apl_home().join("tmp")
}

/// Target CPU architecture for macOS.
///
/// APL supports both Apple Silicon (ARM64) and Intel (x86_64) Macs.
/// The architecture is used to select the correct pre-compiled binary
/// from the package index.
///
/// # Example
///
/// ```
/// use apl::Arch;
///
/// let current = Arch::current();
/// println!("Running on: {}", current);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Arch {
    /// ARM64 architecture (Apple Silicon: M1, M2, M3, etc.)
    Arm64,
    /// x86_64 architecture (Intel Macs)
    X86_64,
}

impl Arch {
    /// Get the current architecture
    pub fn current() -> Self {
        #[cfg(target_arch = "aarch64")]
        {
            Self::Arm64
        }
        #[cfg(not(target_arch = "aarch64"))]
        {
            Self::X86_64
        }
    }

    /// Convert to string representation
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Arm64 => "arm64",
            Self::X86_64 => "x86_64",
        }
    }
}

impl std::fmt::Display for Arch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for Arch {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "arm64" | "aarch64" => Ok(Self::Arm64),
            "x86_64" | "amd64" => Ok(Self::X86_64),
            _ => Err(format!("Unknown architecture: {s}")),
        }
    }
}

/// A normalized package name.
///
/// Package names are automatically lowercased to ensure consistent lookups
/// and comparisons. This prevents issues with case-sensitive package names
/// like `JQ` vs `jq`.
///
/// # Example
///
/// ```
/// use apl::PackageName;
///
/// let name = PackageName::new("JQ");
/// assert_eq!(name.as_str(), "jq");
/// ```
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct PackageName(String);

impl PackageName {
    /// Create a new package name, automatically normalizing to lowercase.
    pub fn new(name: &str) -> Self {
        Self(name.to_lowercase())
    }

    /// Get the normalized package name as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
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

impl PartialEq<&str> for PackageName {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<str> for PackageName {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<String> for PackageName {
    fn eq(&self, other: &String) -> bool {
        &self.0 == other
    }
}

impl From<String> for PackageName {
    fn from(s: String) -> Self {
        Self::new(&s)
    }
}

impl From<&str> for PackageName {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl std::str::FromStr for PackageName {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::new(s))
    }
}

/// A semantic version string.
///
/// Versions are stored as strings to support arbitrary version formats
/// (e.g., `1.2.3`, `2024.01.01`, `nightly`). Comparison and ordering
/// are performed using semantic version parsing where applicable.
///
/// # Example
///
/// ```
/// use apl::Version;
///
/// let version = Version::new("1.7.1");
/// assert_eq!(version.as_str(), "1.7.1");
/// ```
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct Version(String);

impl Version {
    /// Create a new version from a string.
    pub fn new(v: &str) -> Self {
        Self(v.to_string())
    }

    /// Get the version string.
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

impl AsRef<std::ffi::OsStr> for Version {
    fn as_ref(&self) -> &std::ffi::OsStr {
        self.0.as_ref()
    }
}

impl AsRef<std::path::Path> for Version {
    fn as_ref(&self) -> &std::path::Path {
        std::path::Path::new(&self.0)
    }
}

impl PartialEq<&str> for Version {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<str> for Version {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<String> for Version {
    fn eq(&self, other: &String) -> bool {
        &self.0 == other
    }
}

impl From<String> for Version {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<&str> for Version {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

impl std::str::FromStr for Version {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::new(s))
    }
}

/// Newtype for a BLAKE3 hash string (64 hex characters).
///
/// Provides compile-time distinction from other strings and optional runtime validation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Blake3Hash(String);

impl Blake3Hash {
    /// Create a new Blake3Hash without validation (for index/deserialized data).
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// Create a validated Blake3Hash (64 hex characters).
    pub fn validated(s: &str) -> Result<Self, String> {
        if s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()) {
            Ok(Self(s.to_string()))
        } else {
            Err(format!(
                "Invalid BLAKE3 hash: expected 64 hex chars, got '{s}'"
            ))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Blake3Hash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AsRef<str> for Blake3Hash {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<String> for Blake3Hash {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

impl From<&str> for Blake3Hash {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

/// Magic bytes for ZSTD compression (Little Endian: 0xFD2FB528 -> 28 B5 2F FD)
pub const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

/// User Agent string
pub const USER_AGENT: &str = concat!("apl/", env!("CARGO_PKG_VERSION"));
