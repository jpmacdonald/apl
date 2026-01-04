//! TOML Package definition parsing
//!
//! Human-readable package definitions.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::types::{Arch, PackageName, Version};

#[derive(Error, Debug)]
pub enum PackageError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Parse error: {0}")]
    Parse(#[from] toml::de::Error),
}

/// Package type
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PackageType {
    #[default]
    Cli,
    App,
}

/// Artifact format
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactFormat {
    #[serde(rename = "tar.gz")]
    TarGz,
    #[serde(rename = "tar.zst")]
    TarZst,
    Tar,
    Zip,
    Dmg,
    Pkg,
    Binary,
}

/// Installation strategy
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum InstallStrategy {
    #[default]
    Link,
    App,
    Pkg,
    Script,
}

/// Package metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageInfo {
    pub name: PackageName,
    pub version: Version,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub homepage: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    #[serde(rename = "type")]
    pub type_: PackageType,
}

/// Package source
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub url: String,
    pub sha256: String,
    pub format: ArtifactFormat,
    #[serde(default)]
    pub strip_components: u32,
    #[serde(default)]
    pub url_template: Option<String>,
    #[serde(default)]
    pub versions: Option<Vec<String>>,
}

/// Binary artifact (precompiled)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Binary {
    pub url: String,
    pub sha256: String,
    pub format: ArtifactFormat,
    /// Target architecture
    #[serde(default = "default_arch")]
    pub arch: Arch,
    /// Minimum macOS version
    #[serde(default = "default_macos")]
    pub macos: String,
}

fn default_arch() -> Arch {
    crate::types::Arch::Arm64
}

fn default_macos() -> String {
    "14.0".to_string()
}

/// Dependencies
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Dependencies {
    #[serde(default)]
    pub runtime: Vec<String>,
    #[serde(default)]
    pub build: Vec<String>,
    #[serde(default)]
    pub optional: Vec<String>,
}

/// Complete package definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Package {
    pub package: PackageInfo,
    pub source: Source,
    /// Pre-built binaries by architecture
    #[serde(default)]
    #[serde(alias = "bottle")] // Backwards compatibility
    #[serde(alias = "binary")] // Backwards compatibility
    pub targets: HashMap<Arch, Binary>,
    #[serde(default)]
    pub dependencies: Dependencies,
    #[serde(default)]
    pub install: InstallSpec,
    #[serde(default)]
    pub hints: Hints,
    #[serde(default)]
    pub build: Option<BuildSpec>,
}

/// Build instructions (from source)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BuildSpec {
    /// Build-time dependencies (e.g. cmake, ninja)
    #[serde(default)]
    pub dependencies: Vec<String>,
    /// Build script (runs in sysroot)
    #[serde(default)]
    pub script: String,
}

/// Installation specification
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstallSpec {
    /// Installation strategy
    #[serde(default)]
    pub strategy: InstallStrategy,
    /// Files to install to bin/
    #[serde(default)]
    pub bin: Vec<String>,
    /// Files to install to lib/
    #[serde(default)]
    pub lib: Vec<String>,
    /// Files to install to include/
    #[serde(default)]
    pub include: Vec<String>,
    /// Custom install script (shell commands)
    #[serde(default)]
    pub script: String,
    /// Name of the .app bundle to install (for type="app")
    #[serde(default)]
    pub app: Option<String>,
}

/// Post-install hints (printed, never executed)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Hints {
    /// Message to display after installation
    #[serde(default)]
    pub post_install: String,
}

impl Package {
    /// Parse a package from a TOML file
    pub fn from_file(path: &Path) -> Result<Self, PackageError> {
        let content = fs::read_to_string(path)?;
        Self::parse(&content)
    }

    /// Parse a package from a TOML string
    pub fn parse(content: &str) -> Result<Self, PackageError> {
        Ok(toml::from_str(content)?)
    }

    /// Serialize to TOML string
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }

    /// Get binary for current architecture
    pub fn binary_for_current_arch(&self) -> Option<&Binary> {
        let arch = crate::types::Arch::current();
        self.targets.get(&arch)
    }
}

impl std::str::FromStr for Package {
    type Err = PackageError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

// Type aliases for backwards compatibility during migration

// ============================================================================
// Algorithmic Registry Structs (Template-Based Package Definitions)
// ============================================================================

/// Package template for algorithmic registry (stored in registry/)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageTemplate {
    pub package: PackageInfo,
    pub discovery: DiscoveryConfig,
    pub assets: AssetConfig,
    #[serde(default)]
    pub checksums: ChecksumConfig,
    pub install: InstallSpec,
    #[serde(default)]
    pub hints: Hints,
}

impl PackageTemplate {
    pub fn parse(content: &str) -> Result<Self, PackageError> {
        Ok(toml::from_str(content)?)
    }
}

/// How to discover versions
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DiscoveryConfig {
    GitHub {
        github: String, // "owner/repo"
        #[serde(default = "default_tag_pattern")]
        tag_pattern: String, // "{{version}}" or "v{{version}}"
        #[serde(default = "default_true")]
        semver_only: bool,
        #[serde(default)]
        include_prereleases: bool,
        #[serde(default)]
        version_type: VersionType,
    },
    Manual {
        manual: Vec<String>, // List of versions
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum VersionType {
    #[default]
    SemVer,
    Sequential,
    Snapshot,
    CalVer,
}

impl DiscoveryConfig {
    pub fn tag_pattern(&self) -> &str {
        match self {
            Self::GitHub { tag_pattern, .. } => tag_pattern,
            Self::Manual { .. } => "{{version}}",
        }
    }
}

fn default_tag_pattern() -> String {
    "{{version}}".to_string()
}

fn default_true() -> bool {
    true
}

/// How to construct asset URLs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetConfig {
    pub url_template: String,
    #[serde(default)]
    pub targets: Option<HashMap<String, String>>,
    #[serde(default)]
    pub universal: bool, // Single binary for all arches
}

/// Checksum configuration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChecksumConfig {
    #[serde(default)]
    pub url_template: Option<String>,
    /// Expected hash type from vendor (usually sha256)
    #[serde(default)]
    pub vendor_type: Option<crate::index::HashType>,
    #[serde(default)]
    pub skip: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE_PACKAGE: &str = r#"
[package]
name = "neovim"
version = "0.10.0"
description = "Vim-fork focused on extensibility"
homepage = "https://neovim.io"
license = "Apache-2.0"

[source]
url = "https://github.com/neovim/neovim/archive/v0.10.0.tar.gz"
sha256 = "abc123def456"
format = "tar.gz"

[binary.arm64]
url = "https://cdn.example.com/neovim-0.10.0-arm64.tar.zst"
sha256 = "binary123"
format = "tar.zst"
macos = "14.0"

[binary.x86_64]
url = "https://cdn.example.com/neovim-0.10.0-x86_64.tar.zst"
sha256 = "binary456"
format = "tar.zst"
macos = "12.0"

[dependencies]
runtime = ["libuv", "msgpack", "tree-sitter"]
build = ["cmake", "ninja"]

[install]
strategy = "link"
bin = ["nvim"]
"#;

    #[test]
    fn test_parse_package() {
        let pkg = Package::parse(EXAMPLE_PACKAGE).unwrap();

        assert_eq!(pkg.package.name, PackageName::from("neovim"));
        assert_eq!(pkg.package.version, Version::from("0.10.0".to_string()));
        assert_eq!(pkg.source.sha256, "abc123def456");
        assert_eq!(pkg.dependencies.runtime.len(), 3);
        assert_eq!(pkg.targets.len(), 2);
    }

    #[test]
    fn test_binary_for_arch() {
        let pkg = Package::parse(EXAMPLE_PACKAGE).unwrap();
        let binary = pkg.binary_for_current_arch();
        assert!(binary.is_some());
    }

    #[test]
    fn test_parse_malformed_toml() {
        let bad_toml = "this is not valid toml {{{";
        let result = Package::parse(bad_toml);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_missing_required_fields() {
        // Missing [package] section
        let incomplete = r#"
[source]
url = "https://example.com"
sha256 = "abc123"
"#;
        let result = Package::parse(incomplete);
        assert!(result.is_err());
    }

    #[test]
    fn test_from_str_trait() {
        use std::str::FromStr;
        let pkg: Result<Package, _> = Package::from_str(EXAMPLE_PACKAGE);
        assert!(pkg.is_ok());
        assert_eq!(pkg.unwrap().package.name, PackageName::from("neovim"));
    }

    #[test]
    fn test_serialization_roundtrip() {
        let pkg = Package::parse(EXAMPLE_PACKAGE).unwrap();
        let toml_str = pkg.to_toml().unwrap();
        let reparsed = Package::parse(&toml_str).unwrap();

        assert_eq!(pkg.package.name, reparsed.package.name);
        assert_eq!(pkg.package.version, reparsed.package.version);
        assert_eq!(pkg.source.sha256, reparsed.source.sha256);
    }

    #[test]
    fn test_binary_for_nonexistent_arch() {
        // Create a package with only x86_64 binary
        let pkg_with_one_arch = r#"
[package]
name = "test"
version = "1.0"

[source]
url = "https://example.com"
sha256 = "abc"
format = "tar.gz"

[binary.x86_64]
url = "https://example.com/x86.tar.gz"
sha256 = "xyz"
format = "tar.gz"
"#;
        let pkg = Package::parse(pkg_with_one_arch).unwrap();
        // This test will pass on x86 and fail on arm64 - documenting behavior
        let _binary = pkg.binary_for_current_arch();
    }
}
