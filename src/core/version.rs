//! Version parsing and @syntax handling

use anyhow::{Result, bail};

/// Parsed package specifier with optional version
#[derive(Debug, Clone)]
pub struct PackageSpec {
    pub name: String,
    pub version: Option<String>,
}

impl PackageSpec {
    /// Parse a package specifier like "jq" or "jq@1.7.1"
    pub fn parse(spec: &str) -> Result<Self> {
        if let Some((name, version)) = spec.split_once('@') {
            if name.is_empty() {
                bail!("Invalid package specifier: missing package name");
            }
            if version.is_empty() {
                bail!("Invalid package specifier: missing version after @");
            }
            Ok(Self {
                name: name.to_string(),
                version: Some(version.to_string()),
            })
        } else {
            Ok(Self {
                name: spec.to_string(),
                version: None,
            })
        }
    }
    
    /// Check if this specifier requests a specific version
    pub fn is_pinned(&self) -> bool {
        self.version.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple() {
        let spec = PackageSpec::parse("jq").unwrap();
        assert_eq!(spec.name, "jq");
        assert_eq!(spec.version, None);
    }

    #[test]
    fn test_parse_versioned() {
        let spec = PackageSpec::parse("jq@1.7.1").unwrap();
        assert_eq!(spec.name, "jq");
        assert_eq!(spec.version, Some("1.7.1".to_string()));
    }

    #[test]
    fn test_parse_latest() {
        let spec = PackageSpec::parse("jq@latest").unwrap();
        assert_eq!(spec.name, "jq");
        assert_eq!(spec.version, Some("latest".to_string()));
    }

    #[test]
    fn test_parse_invalid() {
        assert!(PackageSpec::parse("@1.0").is_err());
        assert!(PackageSpec::parse("jq@").is_err());
    }
}
