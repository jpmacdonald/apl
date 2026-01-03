//! Robust asset name matching for macOS packages.
//! Handles naming inconsistencies across vendors: macos/darwin/osx, arm64/aarch64, etc.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OsVariant {
    Darwin,
    MacOS,
    Osx,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArchVariant {
    Arm64,
    Aarch64,
    X86_64,
    Amd64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExtVariant {
    TarGz,
    TarXz,
    TarZst,
    Zip,
    Binary,
}

/// A parsed representation of an asset filename's core platform indicators.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetPattern {
    pub os: Option<OsVariant>,
    pub arch: Option<ArchVariant>,
    pub ext: Option<ExtVariant>,
}

impl AssetPattern {
    /// Try to parse semantic meaning from a filename.
    pub fn from_filename(filename: &str) -> Self {
        let f = filename.to_lowercase();

        let os = if f.contains("macos") || f.contains("apple") {
            Some(OsVariant::MacOS)
        } else if f.contains("darwin") {
            Some(OsVariant::Darwin)
        } else if f.contains("osx") {
            Some(OsVariant::Osx)
        } else {
            None
        };

        let arch = if f.contains("arm64") {
            Some(ArchVariant::Arm64)
        } else if f.contains("aarch64") {
            Some(ArchVariant::Aarch64)
        } else if f.contains("x86_64") {
            Some(ArchVariant::X86_64)
        } else if f.contains("amd64") || f.contains("x64") {
            Some(ArchVariant::Amd64)
        } else {
            None
        };

        let ext = if f.ends_with(".tar.gz") || f.ends_with(".tgz") {
            Some(ExtVariant::TarGz)
        } else if f.ends_with(".tar.xz") || f.ends_with(".txz") {
            Some(ExtVariant::TarXz)
        } else if f.ends_with(".tar.zst") {
            Some(ExtVariant::TarZst)
        } else if f.ends_with(".zip") {
            Some(ExtVariant::Zip)
        } else {
            // Check if it looks like a raw binary
            if !f.contains('.') {
                Some(ExtVariant::Binary)
            } else {
                None
            }
        };

        Self { os, arch, ext }
    }

    /// Check if this pattern is semantically equivalent to another.
    /// Used to match expected asset pattern against actual GitHub assets.
    pub fn matches(&self, other: &AssetPattern) -> bool {
        // OS check (Equivalent: MacOS/Darwin/Osx)
        let os_match = match (self.os, other.os) {
            (Some(_), Some(_)) => true, // Any macOS/Darwin variant matches another
            (None, None) => true,
            _ => false,
        };

        // Arch check (Equivalent: Arm64/Aarch64, X86_64/Amd64)
        let arch_match = match (self.arch, other.arch) {
            (Some(a1), Some(a2)) => match (a1, a2) {
                (ArchVariant::Arm64, ArchVariant::Arm64)
                | (ArchVariant::Arm64, ArchVariant::Aarch64)
                | (ArchVariant::Aarch64, ArchVariant::Arm64)
                | (ArchVariant::Aarch64, ArchVariant::Aarch64) => true,
                (ArchVariant::X86_64, ArchVariant::X86_64)
                | (ArchVariant::X86_64, ArchVariant::Amd64)
                | (ArchVariant::Amd64, ArchVariant::X86_64)
                | (ArchVariant::Amd64, ArchVariant::Amd64) => true,
                _ => false,
            },
            (None, None) => true,
            _ => false,
        };

        // Extension check (Fuzzy matching for common archive formats)
        let ext_match = match (self.ext, other.ext) {
            (Some(e1), Some(e2)) => {
                if e1 == e2 {
                    true
                } else {
                    // Accept zip vs tar.gz fallback if specifically requested or common
                    // For now, let's keep it strict or allow common swaps?
                    // User mentioned ".zip vs .tar.gz" in older releases.
                    match (e1, e2) {
                        (ExtVariant::Zip, ExtVariant::TarGz)
                        | (ExtVariant::TarGz, ExtVariant::Zip) => true,
                        _ => false,
                    }
                }
            }
            (None, None) => true,
            _ => false,
        };

        os_match && arch_match && ext_match
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pattern_parsing() {
        let p1 = AssetPattern::from_filename("syncthing-macos-arm64-v1.20.4.zip");
        assert_eq!(p1.os, Some(OsVariant::MacOS));
        assert_eq!(p1.arch, Some(ArchVariant::Arm64));
        assert_eq!(p1.ext, Some(ExtVariant::Zip));

        let p2 = AssetPattern::from_filename("syncthing-darwin-aarch64-v0.14.48.tar.gz");
        assert_eq!(p2.os, Some(OsVariant::Darwin));
        assert_eq!(p2.arch, Some(ArchVariant::Aarch64));
        assert_eq!(p2.ext, Some(ExtVariant::TarGz));
    }

    #[test]
    fn test_pattern_matching() {
        let expected = AssetPattern::from_filename("package-macos-arm64.zip");
        let actual = AssetPattern::from_filename("package-darwin-aarch64.tar.gz");

        assert!(expected.matches(&actual));

        let wrong_arch = AssetPattern::from_filename("package-macos-x86_64.zip");
        assert!(!expected.matches(&wrong_arch));
    }
}
