//! dl - Distill Package Manager
//!
//! A modern, fast package manager for macOS inspired by uv, pacman, and apt.

pub mod cas;
pub mod db;
pub mod downloader;
pub mod extractor;
pub mod formula;
pub mod index;
pub mod lockfile;
pub mod resolver;

use std::path::PathBuf;

use dirs::home_dir;

/// Default dl home directory: ~/.dl
pub fn dl_home() -> PathBuf {
    home_dir()
        .expect("Could not determine home directory")
        .join(".dl")
}

/// Content-addressable store path: ~/.dl/cache
pub fn cas_path() -> PathBuf {
    dl_home().join("cache")
}

/// SQLite database path: ~/.dl/state.db
pub fn db_path() -> PathBuf {
    dl_home().join("state.db")
}

/// Binary installation target: ~/.dl/bin
pub fn bin_path() -> PathBuf {
    dl_home().join("bin")
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
