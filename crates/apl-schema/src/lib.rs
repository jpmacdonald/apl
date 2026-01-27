//! Shared types and wire format for the APL package manager.
//!
//! This crate defines the canonical data structures used across all APL components:
//! the CLI, the build engine, and the package index. It includes schema types for
//! artifacts, package metadata, version specifiers, hash wrappers, architecture
//! detection, asset filename pattern matching, and the binary package index format
//! (Postcard + Zstd).

/// CPU architecture detection and representation for macOS targets.
pub mod arch;
/// Asset filename pattern matching for cross-vendor OS/arch/extension detection.
pub mod asset_pattern;
/// Typed wrappers for cryptographic hashes (SHA-256, BLAKE3).
pub mod hash;
/// Binary package index: serialization, search, and lookup.
pub mod index;
/// Merkle tree for index integrity verification.
pub mod merkle;
/// Core domain types: artifacts, port configs, package names, and versions.
pub mod types;
/// Version parsing, comparison, and requirement matching.
pub mod version;

// Re-exports
pub use arch::*;
pub use hash::*;
pub use index::{IndexEntry, PackageIndex, VersionInfo};
pub use types::*;

/// Magic bytes for ZSTD compression (Little Endian: 0xFD2FB528 -> 28 B5 2F FD)
pub const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

/// Root of Trust: Ed25519 Public Key for Index Verification (Base64)
/// Corresponds to the private key in CI credentials.
pub const APL_PUBLIC_KEY: &str = "rIWQxJ7m4uep6XGvu/SljulxUnHtm2BYfJDUOWxL4Z8=";
