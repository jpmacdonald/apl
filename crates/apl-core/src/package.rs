//! TOML Package definition parsing
//!
//! Human-readable package definitions.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub use crate::types::{
    Arch, ArtifactFormat, BuildSpec, InstallStrategy, PackageName, PackageType, Version,
};

/// Errors that can occur when loading or parsing a package definition.
#[derive(Error, Debug)]
pub enum PackageError {
    /// An I/O error occurred while reading a package file.
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// The TOML content could not be deserialized into a valid package.
    #[error("Parse error: {0}")]
    Parse(#[from] toml::de::Error),
}

/// Metadata describing a package's identity and provenance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageInfo {
    /// Unique name that identifies this package in the registry.
    pub name: PackageName,
    /// Semantic version string for the package release.
    pub version: Version,
    /// Short human-readable summary of the package.
    #[serde(default)]
    pub description: String,
    /// URL of the project's homepage.
    #[serde(default)]
    pub homepage: String,
    /// SPDX license identifier for the package.
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

/// Location and integrity information for a package's source archive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    /// Download URL for the source archive.
    pub url: String,
    /// Expected SHA-256 digest of the downloaded archive.
    pub sha256: String,
    /// Archive format (e.g. `tar.gz`, `zip`).
    pub format: ArtifactFormat,
    /// Number of leading path components to strip when extracting.
    #[serde(default)]
    pub strip_components: Option<u32>,
}

/// A precompiled binary artifact for a specific platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Binary {
    /// Download URL for the binary artifact.
    pub url: String,
    /// Expected SHA-256 digest of the downloaded artifact.
    pub sha256: String,
    /// Archive format of the binary artifact.
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

/// Dependency lists grouped by when they are required.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Dependencies {
    /// Packages required at runtime.
    #[serde(default)]
    pub runtime: Vec<String>,
    /// Packages required only during the build phase.
    #[serde(default)]
    pub build: Vec<String>,
    /// Packages that are optional and provide extra functionality.
    #[serde(default)]
    pub optional: Vec<String>,
}

/// Complete package definition combining metadata, source, dependencies,
/// install instructions, and user-facing hints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Package {
    /// Core metadata for the package (name, version, description, etc.).
    pub package: PackageInfo,
    /// Source archive location and integrity data.
    pub source: Source,
    /// Runtime, build, and optional dependency lists.
    #[serde(default)]
    pub dependencies: Dependencies,
    /// Instructions describing how to install the package after extraction.
    #[serde(default)]
    pub install: InstallSpec,
    /// Post-install messages displayed to the user.
    #[serde(default)]
    pub hints: Hints,
    /// Optional build specification for compiling from source.
    #[serde(default)]
    pub build: Option<BuildSpec>,
}

/// Installation specification controlling how extracted artifacts are placed
/// into the sysroot.
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
    /// Returns the effective list of binaries to install, falling back to an
    /// empty list when none are explicitly configured.
    pub fn effective_bin(&self, _pkg_name: &str) -> Vec<String> {
        self.bin.clone().unwrap_or_default()
    }

    /// Returns the effective install strategy, defaulting to `App` when an
    /// `.app` bundle is specified, or `Link` otherwise.
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
    /// Parse a package definition from a TOML file on disk.
    ///
    /// # Errors
    ///
    /// Returns `PackageError::Io` if the file cannot be read, or
    /// `PackageError::Parse` if the TOML content is invalid.
    pub fn from_file(path: &Path) -> Result<Self, PackageError> {
        let content = fs::read_to_string(path)?;
        Self::parse(&content)
    }

    /// Parse a package definition from a TOML string.
    ///
    /// # Errors
    ///
    /// Returns `PackageError::Parse` if the TOML content is invalid or does
    /// not match the expected schema.
    pub fn parse(content: &str) -> Result<Self, PackageError> {
        Ok(toml::from_str(content)?)
    }

    /// Serialize this package definition to a pretty-printed TOML string.
    ///
    /// # Errors
    ///
    /// Returns a `toml::ser::Error` if serialization fails.
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

/// Template-based package definition stored in the algorithmic registry.
///
/// Unlike a concrete `Package`, a template uses discovery rules and asset
/// selectors to automatically generate versioned package definitions from
/// upstream release metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageTemplate {
    /// Template-level metadata (name, description, etc.).
    pub package: PackageInfoTemplate,
    /// Rules for discovering available versions from upstream sources.
    pub discovery: DiscoveryConfig,
    /// Configuration for selecting downloadable assets per architecture.
    #[serde(default)]
    pub assets: AssetConfig,
    /// Optional source-archive template for building from source.
    #[serde(default)]
    pub source: Option<SourceTemplate>,
    /// Optional build specification for compiling from source.
    #[serde(default)]
    pub build: Option<BuildSpec>,
    /// Runtime, build, and optional dependency lists.
    #[serde(default)]
    pub dependencies: Dependencies,
    /// Instructions describing how to install the package.
    pub install: InstallSpec,
    /// Post-install hints displayed to the user.
    #[serde(default)]
    pub hints: Hints,
}

/// Source archive template with URL placeholders for version substitution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceTemplate {
    /// URL template for source code (e.g. `"{{github}}/archive/refs/tags/{{tag}}.tar.gz"`).
    pub url: String,
    /// Expected format of the source archive.
    pub format: ArtifactFormat,
    /// Verification hash for the source archive (optional, defaults to computing on first fetch).
    pub sha256: Option<String>,
}

/// Metadata template for a package in the algorithmic registry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageInfoTemplate {
    /// Unique name that identifies this package.
    pub name: PackageName,
    /// Short human-readable summary of the package.
    #[serde(default)]
    pub description: String,
    /// URL of the project's homepage.
    #[serde(default)]
    pub homepage: String,
    /// SPDX license identifier.
    #[serde(default)]
    pub license: String,
    /// Category tags for the package.
    #[serde(default)]
    pub tags: Vec<String>,
}

impl PackageTemplate {
    /// Parse a `PackageTemplate` from a TOML string.
    ///
    /// # Errors
    ///
    /// Returns `PackageError::Parse` if the TOML content is invalid or does
    /// not match the expected template schema.
    pub fn parse(content: &str) -> Result<Self, PackageError> {
        Ok(toml::from_str(content)?)
    }
}

/// Configuration for how upstream versions are discovered.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DiscoveryConfig {
    /// Discover versions from GitHub releases for the given `owner/repo`.
    GitHub {
        /// GitHub repository in `"owner/repo"` format.
        github: String, // "owner/repo"
        /// Tag pattern used to extract version strings (e.g. `"v{{version}}"`).
        #[serde(default = "default_tag_pattern")]
        tag_pattern: String, // "{{version}}" or "v{{version}}"
        /// Whether to include pre-release versions.
        #[serde(default)]
        include_prereleases: bool,
    },
    /// Discover versions from a ports-style source (e.g. `"ruby"`).
    Ports {
        /// Name of the ports package to query.
        #[serde(rename = "ports")]
        name: String, // e.g. "ruby"
    },
    /// A manually curated list of version strings.
    Manual {
        /// Explicit list of version strings.
        manual: Vec<String>, // List of versions
    },
}

impl DiscoveryConfig {
    /// Returns the GitHub `"owner/repo"` string if this is a `GitHub` variant,
    /// or `None` otherwise.
    pub fn github_repo(&self) -> Option<&str> {
        match self {
            DiscoveryConfig::GitHub { github, .. } => Some(github),
            _ => None,
        }
    }
}

fn default_tag_pattern() -> String {
    "{{version}}".to_string()
}

/// Rules for selecting the correct downloadable asset for a given platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AssetSelector {
    /// Auto-detect using typed pattern matching (recommended).
    Auto {
        /// When `true`, enables automatic asset detection.
        auto: bool,
    },
    /// Match files whose name ends with the given suffix
    /// (e.g. `"x86_64-apple-darwin.tar.gz"`).
    Suffix {
        /// The suffix string to match against asset filenames.
        suffix: String,
    },
    /// Match files whose name matches the given regular expression.
    Regex {
        /// The regular expression pattern to match asset filenames.
        regex: String,
    },
    /// Indicates the package must be built from source (hydration).
    Build {
        /// When `true`, marks the asset as requiring a source build.
        build: bool,
    },
    /// Match by exact filename.
    Exact {
        /// The exact filename to match.
        name: String,
    },
}

/// Configuration for constructing and verifying asset download URLs.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AssetConfig {
    /// Explicit selectors for each architecture.
    /// Flattened so they appear directly under `[assets]` in TOML.
    #[serde(flatten)]
    pub select: HashMap<String, AssetSelector>,

    /// When `true`, skip checksum verification for downloaded assets.
    #[serde(default)]
    pub skip_checksums: bool,

    /// URL template for an external checksum file, if the repository does not
    /// embed checksums in its release metadata.
    #[serde(default)]
    pub checksum_url: Option<String>,
}

// /// Installation specification with optional fields for inference
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
    fn test_parse_package_template_with_dependencies() {
        let template_toml = r#"
[package]
name = "test-pkg"
description = "test"
homepage = "https://example.com"

[discovery]
manual = ["1.0.0"]

[assets]
skip_checksums = true
universal-macos = { suffix = "bin" }

[dependencies]
runtime = ["lima"]
build = ["cargo"]

[install]
bin = ["test"]
"#;
        let template = PackageTemplate::parse(template_toml).unwrap();
        assert_eq!(
            template.package.name,
            PackageName::from("test-pkg".to_string())
        );
        assert_eq!(template.dependencies.runtime, vec!["lima"]);
        assert_eq!(template.dependencies.build, vec!["cargo"]);
    }
}
