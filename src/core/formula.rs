//! TOML Formula parsing
//!
//! Human-readable package definitions.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum FormulaError {
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

/// Package metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageInfo {
    pub name: String,
    pub version: String,
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
    pub blake3: String,
    #[serde(default)]
    pub strip_components: u32,
}

/// Bottle (precompiled binary)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bottle {
    pub url: String,
    pub blake3: String,
    /// Target architecture: "arm64" or "x86_64"
    #[serde(default = "default_arch")]
    pub arch: String,
    /// Minimum macOS version
    #[serde(default = "default_macos")]
    pub macos: String,
}

fn default_arch() -> String {
    crate::arch::ARM64.to_string()
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

/// Complete package formula
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Formula {
    pub package: PackageInfo,
    pub source: Source,
    #[serde(default)]
    pub bottle: HashMap<String, Bottle>,
    #[serde(default)]
    pub dependencies: Dependencies,
    #[serde(default)]
    pub install: InstallSpec,
    #[serde(default)]
    pub hints: Hints,
}

/// Installation specification
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstallSpec {
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

impl Formula {
    /// Parse a formula from a TOML file
    pub fn from_file(path: &Path) -> Result<Self, FormulaError> {
        let content = fs::read_to_string(path)?;
        Self::from_str(&content)
    }

    /// Parse a formula from a TOML string
    pub fn from_str(content: &str) -> Result<Self, FormulaError> {
        Ok(toml::from_str(content)?)
    }

    /// Serialize to TOML string
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }

    /// Get bottle for current architecture
    pub fn bottle_for_current_arch(&self) -> Option<&Bottle> {
        let arch = crate::arch::current();
        self.bottle.get(arch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE_FORMULA: &str = r#"
[package]
name = "neovim"
version = "0.10.0"
description = "Vim-fork focused on extensibility"
homepage = "https://neovim.io"
license = "Apache-2.0"

[source]
url = "https://github.com/neovim/neovim/archive/v0.10.0.tar.gz"
blake3 = "abc123def456"

[bottle.arm64]
url = "https://cdn.example.com/neovim-0.10.0-arm64.tar.zst"
blake3 = "bottle123"
macos = "14.0"

[bottle.x86_64]
url = "https://cdn.example.com/neovim-0.10.0-x86_64.tar.zst"
blake3 = "bottle456"
macos = "12.0"

[dependencies]
runtime = ["libuv", "msgpack", "tree-sitter"]
build = ["cmake", "ninja"]

[install]
bin = ["nvim"]
"#;

    #[test]
    fn test_parse_formula() {
        let formula = Formula::from_str(EXAMPLE_FORMULA).unwrap();

        assert_eq!(formula.package.name, "neovim");
        assert_eq!(formula.package.version, "0.10.0");
        assert_eq!(formula.source.blake3, "abc123def456");
        assert_eq!(formula.dependencies.runtime.len(), 3);
        assert_eq!(formula.bottle.len(), 2);
    }

    #[test]
    fn test_bottle_for_arch() {
        let formula = Formula::from_str(EXAMPLE_FORMULA).unwrap();
        let bottle = formula.bottle_for_current_arch();
        assert!(bottle.is_some());
    }
}
