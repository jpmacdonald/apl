pub mod arch;
pub mod asset_pattern;
pub mod hash;
pub mod index;
pub mod merkle;
pub mod types;
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
pub const APL_PUBLIC_KEY: &str = "ltcv47HgxDoAAXi+BVJ9FL3i93jkGa+pAs9leVKqUu4=";
