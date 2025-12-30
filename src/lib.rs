//! apl - A Package Layer
//!
//! A fast, minimal package manager for macOS CLI tools.

pub mod core;
pub mod io;
pub mod store;

// Re-exports for convenience
pub use core::index;
pub use core::lockfile;
pub use core::package;
pub use core::resolver;
pub use io::download as downloader;
pub use io::extract as extractor;
pub use store::cas;
pub use store::db;

// Backwards compatibility
pub use core::package as formula;

use dirs::home_dir;
use std::path::PathBuf;

/// Try to get the apl home directory, returning None if home directory cannot be determined.
pub fn try_apl_home() -> Option<PathBuf> {
    home_dir().map(|h| h.join(".apl"))
}

/// Default apl home directory: ~/.apl
///
/// # Panics
/// Panics if the home directory cannot be determined.
pub fn apl_home() -> PathBuf {
    try_apl_home().expect("Could not determine home directory")
}

/// Content-addressable store path: ~/.apl/cache
pub fn cas_path() -> PathBuf {
    apl_home().join("cache")
}

/// SQLite database path: ~/.apl/state.db
pub fn db_path() -> PathBuf {
    apl_home().join("state.db")
}

/// Binary installation target: ~/.apl/bin
pub fn bin_path() -> PathBuf {
    apl_home().join("bin")
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
