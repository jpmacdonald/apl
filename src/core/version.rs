//! Version parsing - simple exact versions only
//!
//! Supports:
//! - Latest: `jq` or `jq@latest`
//! - Exact: `jq@1.7.1`

use anyhow::{Result, bail};

/// Parsed package specifier with optional version
#[derive(Debug, Clone)]
pub struct PackageSpec {
    pub name: String,
    pub version: Option<String>,
}

impl PackageSpec {
    /// Parse a package specifier like `jq` or `jq@1.7.1`
    pub fn parse(spec: &str) -> Result<Self> {
        if let Some((name, version)) = spec.split_once('@') {
            if name.is_empty() {
                bail!("Invalid package specifier: missing package name");
            }
            if version.is_empty() {
                bail!("Invalid package specifier: missing version after @");
            }

            // Treat "latest" as no version (get latest)
            let version = if version == "latest" {
                None
            } else {
                Some(version.to_string())
            };

            Ok(Self {
                name: name.to_string(),
                version,
            })
        } else {
            Ok(Self {
                name: spec.to_string(),
                version: None,
            })
        }
    }

    /// Get version string for display
    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }

    /// Check if this specifier requests a specific version
    pub fn is_pinned(&self) -> bool {
        self.version.is_some()
    }
}

/// Compare two semantic versions. Returns true if `latest` is newer than `current`.
/// Handles simple numeric comparison (e.g. 1.2.3 > 1.2.2).
pub fn is_newer(current: &str, latest: &str) -> bool {
    let parse =
        |v: &str| -> Vec<u32> { v.split('.').filter_map(|s| s.parse::<u32>().ok()).collect() };

    let c_parts = parse(current);
    let l_parts = parse(latest);

    for i in 0..std::cmp::max(c_parts.len(), l_parts.len()) {
        let cv = c_parts.get(i).unwrap_or(&0);
        let lv = l_parts.get(i).unwrap_or(&0);
        if lv > cv {
            return true;
        }
        if cv > lv {
            return false;
        }
    }
    false
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
        assert_eq!(spec.version, None); // latest = no version = get latest
    }

    #[test]
    fn test_parse_invalid() {
        assert!(PackageSpec::parse("@1.0").is_err());
        assert!(PackageSpec::parse("jq@").is_err());
    }

    #[test]
    fn test_is_pinned() {
        let pinned = PackageSpec::parse("jq@1.7.1").unwrap();
        assert!(pinned.is_pinned());

        let unpinned = PackageSpec::parse("jq").unwrap();
        assert!(!unpinned.is_pinned());

        let latest = PackageSpec::parse("jq@latest").unwrap();
        assert!(!latest.is_pinned()); // @latest is not pinned
    }

    #[test]
    fn test_version_method() {
        let spec = PackageSpec::parse("jq@1.7.1").unwrap();
        assert_eq!(spec.version(), Some("1.7.1"));

        let spec2 = PackageSpec::parse("jq").unwrap();
        assert_eq!(spec2.version(), None);
    }

    #[test]
    fn test_is_newer() {
        assert!(is_newer("1.2.3", "1.2.4"));
        assert!(is_newer("1.2.3", "1.3.0"));
        assert!(is_newer("1.2.3", "2.0.0"));
        assert!(is_newer("0.10.4", "0.11.5"));
        assert!(!is_newer("1.2.3", "1.2.3"));
        assert!(!is_newer("1.2.3", "1.2.2"));
        assert!(!is_newer("1.2.3", "1.1.5"));
        assert!(!is_newer("1.11.5", "1.10.4"));
    }
}
