//! apl - A Package Layer
//!
//! Fast, minimal package manager for macOS CLI tools.

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
    log_dir().join(format!("build-{}-{}-{}.log", package, version, timestamp))
}

/// Temp path: ~/.apl/tmp (guaranteed same volume as store)
pub fn tmp_path() -> PathBuf {
    apl_home().join("tmp")
}

/// Architecture constants
pub mod arch {
    /// ARM64 architecture (Apple Silicon)
    pub const ARM64: &str = "arm64";
    /// x86_64 architecture (Intel)
    pub const X86_64: &str = "x86_64";

    /// Get the current architecture string
    pub fn current() -> &'static str {
        if cfg!(target_arch = "aarch64") {
            ARM64
        } else {
            X86_64
        }
    }
}

/// Magic bytes for ZSTD compression (Little Endian: 0xFD2FB528 -> 28 B5 2F FD)
pub const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

/// User Agent string
pub const USER_AGENT: &str = concat!("apl/", env!("CARGO_PKG_VERSION"));
