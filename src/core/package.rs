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
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
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
    /// Categories/Tags
    #[serde(default)]
    pub tags: Vec<String>,
    /// Internal only: Used after resolution to know which installer type to use.
    /// Not present in registry TOMLs.
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_: Option<PackageType>,
}

/// Package source
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub url: String,
    pub sha256: String,
    pub format: ArtifactFormat,
    #[serde(default)]
    pub strip_components: Option<u32>,
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
    /// Installation strategy (inferred from fields if missing)
    #[serde(default)]
    pub strategy: Option<InstallStrategy>,
    /// Files to install to bin/ (defaults to package name if strategy is Link)
    #[serde(default)]
    pub bin: Option<Vec<String>>,
    /// Files to install to lib/
    #[serde(default)]
    pub lib: Vec<String>,
    /// Files to install to include/
    #[serde(default)]
    pub include: Vec<String>,
    /// Custom install script (shell commands)
    #[serde(default)]
    pub script: Option<String>,
    /// Name of the .app bundle to install
    #[serde(default)]
    pub app: Option<String>,
}

impl InstallSpec {
    pub fn effective_bin(&self, pkg_name: &str) -> Vec<String> {
        self.bin
            .clone()
            .unwrap_or_else(|| vec![pkg_name.to_string()])
    }

    pub fn effective_strategy(&self) -> InstallStrategy {
        self.strategy.clone().unwrap_or_else(|| {
            if self.app.is_some() {
                InstallStrategy::App
            } else {
                InstallStrategy::Link
            }
        })
    }
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
    pub package: PackageInfoTemplate,
    pub discovery: DiscoveryConfig,
    pub assets: AssetConfig,
    #[serde(default)]
    pub source: Option<SourceTemplate>,
    #[serde(default)]
    pub build: Option<BuildSpec>,
    pub install: InstallSpec,
    #[serde(default)]
    pub hints: Hints,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceTemplate {
    /// URL template for source code (e.g. "{{github}}/archive/refs/tags/{{tag}}.tar.gz")
    pub url: String,
    /// Expected format of the source archive
    pub format: ArtifactFormat,
    /// Verification hash for the source archive (optional, defaults to computing on first fetch)
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageInfoTemplate {
    pub name: PackageName,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub homepage: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub tags: Vec<String>,
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
        #[serde(default)]
        include_prereleases: bool,
    },
    Manual {
        manual: Vec<String>, // List of versions
    },
}

fn default_tag_pattern() -> String {
    "{{version}}".to_string()
}

/// Asset selection rules
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AssetSelector {
    /// Match files ending with this string (e.g. "x86_64-apple-darwin.tar.gz")
    Suffix { suffix: String },
    /// Match files matching this regex
    Regex { regex: String },
    /// Exact filename match
    Exact { name: String },
}

/// How to construct asset URLs
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetConfig {
    /// Support for tools with a single multi-arch binary (e.g. shell scripts)
    #[serde(default)]
    pub universal: bool,

    /// Explicit selectors for each architecture.
    /// Flattened so they appear directly under [assets] in TOML.
    #[serde(flatten)]
    pub select: HashMap<String, AssetSelector>,

    /// Optional: Use if the repo doesn't provide checksums
    #[serde(default)]
    pub skip_checksums: bool,

    /// Optional: URL template for external checksum file
    #[serde(default)]
    pub checksum_url: Option<String>,
}

/// Installation specification with optional fields for inference
// (Removed duplicate InstallSpec definition)

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

[dependencies]
runtime = ["libuv", "msgpack", "tree-sitter"]
build = ["cmake", "ninja"]

[install]
bin = ["nvim"]
"#;

    #[test]
    fn test_parse_package() {
        let pkg = Package::parse(EXAMPLE_PACKAGE).unwrap();

        assert_eq!(pkg.package.name, PackageName::from("neovim"));
        assert_eq!(pkg.package.version, Version::from("0.10.0".to_string()));
        assert_eq!(pkg.source.sha256, "abc123def456");
        assert_eq!(pkg.dependencies.runtime.len(), 3);
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
}
