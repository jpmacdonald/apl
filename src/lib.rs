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
//! - **Newtypes**: `PackageName`, `Version`, and `Sha256Hash` provide type-safe identifiers.
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
pub mod indexer;
pub mod io;
pub mod ops;
pub mod store;
pub mod types;
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
    if let Ok(val) = std::env::var("APL_HOME") {
        return Some(PathBuf::from(val));
    }
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

/// Extract the filename from a URL.
///
/// # Example
///
/// ```
/// use apl::filename_from_url;
///
/// assert_eq!(filename_from_url("https://example.com/path/to/file.tar.gz"), "file.tar.gz");
/// assert_eq!(filename_from_url(""), "");
/// ```
pub fn filename_from_url(url: &str) -> &str {
    url.split('/').next_back().unwrap_or("")
}

/// Magic bytes for ZSTD compression (Little Endian: 0xFD2FB528 -> 28 B5 2F FD)
pub const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

/// User Agent string
pub const USER_AGENT: &str = concat!("apl/", env!("CARGO_PKG_VERSION"));

/// Root of Trust: Ed25519 Public Key for Index Verification (Base64)
/// Corresponds to the private key in CI credentials.
pub const APL_PUBLIC_KEY: &str = "8OZpudmmAQrRd7M2XqZKO3VhHyIrbtn3S4p0AlSApT0=";
