//! apl - A Package Layer
//!
//! A fast, minimal package manager for macOS CLI tools.

pub mod core;
pub mod io;
pub mod store;

// Re-exports for convenience
pub use core::package;
pub use core::index;
pub use core::lockfile;
pub use core::resolver;
pub use store::cas;
pub use store::db;
pub use io::download as downloader;
pub use io::extract as extractor;

// Backwards compatibility
pub use core::package as formula;

use std::path::PathBuf;
use dirs::home_dir;

/// Default apl home directory: ~/.apl
pub fn apl_home() -> PathBuf {
    home_dir()
        .expect("Could not determine home directory")
        .join(".apl")
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
