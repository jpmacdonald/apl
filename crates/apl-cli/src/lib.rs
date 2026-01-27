//! apl - A Package Layer
#![allow(missing_docs)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::doc_markdown)]
#![allow(clippy::missing_panics_doc)]
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
//! ├── index   # Binary package index
//! └── state.db    # SQLite database
//! ```

pub mod cmd;
pub mod ops;
pub mod store;
pub mod ui;

// Re-exports from other crates for convenience/compatibility
pub use crate::store::DbHandle;
pub use crate::store::db;
pub use apl_core::Strategy;
pub use apl_core::io::download as downloader;
pub use apl_core::io::extract as extractor;
pub use apl_core::package::{self, Package};
pub use apl_core::resolver;
pub use apl_schema::index;

pub use apl_core::paths::*;

/// Extract the filename from a URL.
///
/// # Example
///
/// ```
/// use apl_cli::filename_from_url;
///
/// assert_eq!(filename_from_url("https://example.com/path/to/file.tar.gz"), "file.tar.gz");
/// assert_eq!(filename_from_url(""), "");
/// ```
pub fn filename_from_url(url: &str) -> &str {
    url.split('/').next_back().unwrap_or("")
}

/// Magic bytes for ZSTD compression
pub use apl_schema::ZSTD_MAGIC;

/// User Agent string (re-exported from apl_core)
pub use apl_core::USER_AGENT;

/// Root of Trust: Ed25519 Public Key for Index Verification (Base64)
pub use apl_schema::APL_PUBLIC_KEY;

use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(name = "apl")]
#[command(author, version, about = "apl - A Package Layer for macOS")]
pub struct Cli {
    /// Show what would happen without making changes
    #[arg(long, global = true)]
    pub dry_run: bool,

    /// Suppress non-essential output
    #[arg(short, long, global = true)]
    pub quiet: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Install a package
    Install {
        /// Package name(s), optionally with version: pkg or pkg@1.0.0
        #[arg(required = true)]
        packages: Vec<String>,
        /// Show verbose output (DMG mounting, file counts, etc.)
        #[arg(short, long)]
        verbose: bool,
    },
    /// Remove a package
    Remove {
        /// Package name(s)
        #[arg(required_unless_present = "all")]
        packages: Vec<String>,
        /// Remove all installed packages
        #[arg(long, short = 'a', conflicts_with = "packages")]
        all: bool,
        /// Skip confirmation prompt
        #[arg(long, short = 'y')]
        yes: bool,
        /// Force removal of package metadata even if files are missing
        #[arg(long, short = 'f')]
        force: bool,
    },
    /// Switch active version of a package
    Use {
        /// Package spec (e.g. jq@1.6)
        spec: String,
    },
    /// View package history
    History {
        /// Package name
        package: String,
    },
    /// Rollback package to previous state
    Rollback {
        /// Package name
        package: String,
    },
    /// List installed packages
    List,
    /// Show package info
    Info {
        /// Package name
        package: String,
    },
    /// Compute SHA256 hash of a file (for package authoring)
    #[command(hide = true)]
    Hash {
        /// Files to hash
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    /// Search available packages
    Search {
        /// Search query
        query: String,
    },
    /// Remove orphaned CAS blobs and temp files
    Clean,
    /// Update package index from CDN
    Update {
        /// CDN URL for index
        #[arg(long, env = "APL_INDEX_URL", default_value = "https://apl.pub/index")]
        url: String,
        /// Upgrade all installed packages after updating index
        #[arg(long)]
        all: bool,
    },
    /// Upgrade installed packages to latest versions
    Upgrade {
        /// Specific packages to upgrade (or all if empty)
        packages: Vec<String>,
        /// Skip confirmation prompt
        #[arg(short, long)]
        yes: bool,
    },
    /// Check status of installed packages
    Status,
    /// Package management commands
    Package {
        #[command(subcommand)]
        command: PackageCommands,
    },
    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        shell: clap_complete::Shell,
    },
    /// Update apl itself to the latest version
    #[command(name = "self-update")]
    SelfUpdate,
    /// Run a package without installing it globally
    Run {
        /// Package name
        package: String,
        /// Arguments for the package
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Enter a project-scoped shell environment
    Shell {
        /// Fail if lockfile is missing or out of sync (for CI)
        #[arg(long)]
        frozen: bool,
        /// Force re-resolution even if lockfile is valid
        #[arg(long)]
        update: bool,
        /// Optional command to run inside the shell
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Option<Vec<String>>,
    },
}

#[derive(Subcommand, Debug)]
pub enum PackageCommands {
    /// Create a new package template
    New {
        /// Package name
        name: String,
        /// Directory to save the package in
        #[arg(long, default_value = "packages")]
        output_dir: PathBuf,
    },
    /// Validate a package file
    Check {
        /// Package file to check
        path: PathBuf,
    },
    /// Bump a package version
    Bump {
        /// Package file to check
        path: PathBuf,
        /// New version
        #[arg(long)]
        version: String,
        /// New binary URL for current arch
        #[arg(long)]
        url: String,
    },
}
