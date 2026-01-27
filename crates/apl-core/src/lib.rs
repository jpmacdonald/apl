//! Core engine for the APL package manager.
//!
//! This crate provides the foundational logic for package resolution, installation,
//! building, indexing, and system root management. It is designed to be consumed
//! by higher-level CLI or GUI frontends without coupling to any specific UI.

/// Package building subsystem for compiling packages from source.
pub mod builder;
/// Indexing subsystem for discovering and cataloging available packages.
pub mod indexer;
/// I/O utilities for downloading, extracting, and verifying artifacts.
pub mod io;
/// Manifest and lockfile parsing for project-level dependency declarations.
pub mod manifest;
/// TOML-based package definition parsing and serialization.
pub mod package;
/// Filesystem path helpers for the APL directory layout.
pub mod paths;
/// `PubGrub`-based dependency resolution adapter.
pub mod pubgrub_adapter;
/// Binary relinking utilities for adjusting Mach-O load commands.
pub mod relinker;
/// Repository management for package registries.
pub mod repo;
/// High-level dependency resolver orchestrating `PubGrub` with APL metadata.
pub mod resolver;
/// Artifact discovery strategies for upstream package sources.
pub mod strategies;
/// Sysroot management for isolated package installation prefixes.
pub mod sysroot;
/// Shared type aliases and re-exports used throughout the crate.
pub mod types;

/// Progress reporting trait and implementations for UI decoupling.
pub mod reporter;

// Re-export Strategy trait if needed, or let users import from strategies
pub use paths::*;
pub use reporter::{NullReporter, Reporter};
pub use strategies::Strategy;

/// User Agent string for core operations.
pub const USER_AGENT: &str = concat!("apl-core/", env!("CARGO_PKG_VERSION"));
