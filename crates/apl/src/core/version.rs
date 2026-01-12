//! Version parsing - simple exact versions only
//!
//! Supports:
//! - Latest: `jq` or `jq@latest`
//! - Exact: `jq@1.7.1`

use anyhow::{Result, bail};

use crate::types::{PackageName, Version};

/// Parsed package specifier with optional version
#[derive(Debug, Clone)]
pub struct PackageSpec {
    pub name: PackageName,
    pub version: Option<Version>,
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
                Some(Version::from(version.to_string()))
            };

            Ok(Self {
                name: PackageName::from(name.to_string()),
                version,
            })
        } else {
            Ok(Self {
                name: PackageName::from(spec.to_string()),
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

    // If numeric parts are identical, check for pre-release suffixes.
    // Logic: Stable (no suffix) > Pre-release (any suffix).
    // e.g. 1.0.0 > 1.0.0-beta
    //
    // If 'latest' has no suffix and 'current' has a suffix, latest is newer (upgrade to stable).
    // If 'current' has no suffix and 'latest' has a suffix, latest is OLDER (don't downgrade to beta).
    //
    // Note: This simple check doesn't compare "beta.1" vs "beta.2", but it solves the
    // "stuck on beta" problem when stable is out.
    let has_suffix = |v: &str| v.contains('-') || v.chars().any(|c| c.is_alphabetic());

    let c_has_suffix = has_suffix(current);
    let l_has_suffix = has_suffix(latest);

    if c_has_suffix && !l_has_suffix {
        // Current is beta, latest is stable -> Newer
        return true;
    }
    if !c_has_suffix && l_has_suffix {
        // Current is stable, latest is beta -> Older (ignore)
        return false;
    }

    // Fallback: If both have suffixes, use lexicographical comparison as a heuristic.
    // e.g. "1.0.0-beta.2" > "1.0.0-beta.1"
    // e.g. "1.0.0-rc.1" > "1.0.0-beta.1"
    if c_has_suffix && l_has_suffix {
        return latest > current;
    }

    // Fallback: Same version or unknown
    false
}

/// Check if a version satisfies a requirement using semver.
/// Falls back to segment-based prefix matching for non-semver specs.
pub fn version_satisfies_requirement(version: &str, requirement: &str) -> bool {
    // Handle "latest" and "*" specially
    if requirement == "latest" || requirement == "*" {
        return true;
    }

    // Exact match
    if version == requirement {
        return true;
    }

    // Try semver parsing
    if let (Ok(ver), Ok(req)) = (
        semver::Version::parse(version),
        semver::VersionReq::parse(requirement),
    ) {
        return req.matches(&ver);
    }

    // Fallback: segment-based prefix match
    version_matches_segments(version, requirement)
}

/// Check if a version string matches a requirement by comparing segments.
/// "0.2" matches "0.2.0", "0.2.1" but NOT "0.20.0"
pub fn version_matches_segments(version: &str, requirement: &str) -> bool {
    let v_parts: Vec<&str> = version.split('.').collect();
    let r_parts: Vec<&str> = requirement.split('.').collect();

    // Requirement must not have more segments than version
    if r_parts.len() > v_parts.len() {
        return false;
    }

    // Each segment of the requirement must match the corresponding version segment
    r_parts.iter().zip(v_parts.iter()).all(|(r, v)| r == v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple() {
        let spec = PackageSpec::parse("jq").unwrap();
        assert_eq!(spec.name, PackageName::from("jq"));
        assert_eq!(spec.version, None);
    }

    #[test]
    fn test_parse_versioned() {
        let spec = PackageSpec::parse("jq@1.7.1").unwrap();
        assert_eq!(spec.name, PackageName::from("jq"));
        assert_eq!(spec.version, Some(Version::from("1.7.1".to_string())));
    }

    #[test]
    fn test_parse_latest() {
        let spec = PackageSpec::parse("jq@latest").unwrap();
        assert_eq!(spec.name, PackageName::from("jq"));
        assert_eq!(spec.version, None);
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
    }

    #[test]
    fn test_version_method() {
        let spec = PackageSpec::parse("jq@1.7.1").unwrap();
        assert_eq!(spec.version(), Some("1.7.1"));
        let spec2 = PackageSpec::parse("jq").unwrap();
        assert_eq!(spec2.version(), None);
    }

    #[test]
    fn test_is_newer_numeric() {
        assert!(is_newer("1.2.3", "1.2.4"));
        assert!(is_newer("1.2.3", "1.3.0"));
        assert!(!is_newer("1.2.3", "1.2.2"));
    }

    #[test]
    fn test_is_newer_prerelease_upgrade() {
        assert!(is_newer("1.0.0-beta", "1.0.0"));
        assert!(!is_newer("1.0.0", "1.0.0-beta"));
    }

    #[test]
    fn test_is_newer_intra_prerelease() {
        assert!(is_newer("1.0.0-beta.1", "1.0.0-beta.2"));
        assert!(is_newer("1.0.0-alpha", "1.0.0-beta"));
        assert!(is_newer("1.0.0-beta", "1.0.0-rc.1"));
        assert!(!is_newer("1.0.0-beta.2", "1.0.0-beta.1"));
    }

    #[test]
    fn test_version_satisfies_requirement_exact() {
        assert!(version_satisfies_requirement("1.2.3", "1.2.3"));
        assert!(!version_satisfies_requirement("1.2.3", "1.2.4"));
    }

    #[test]
    fn test_version_satisfies_requirement_latest() {
        assert!(version_satisfies_requirement("1.2.3", "latest"));
        assert!(version_satisfies_requirement("0.0.1", "*"));
    }

    #[test]
    fn test_version_satisfies_requirement_semver() {
        assert!(version_satisfies_requirement("0.26.1", "^0.26"));
        assert!(version_satisfies_requirement("0.26.0", "^0.26"));
        assert!(!version_satisfies_requirement("0.27.0", "^0.26"));
    }

    #[test]
    fn test_version_satisfies_requirement_prefix() {
        assert!(version_satisfies_requirement("20.12.0", "20"));
        assert!(version_satisfies_requirement("20.0.0", "20"));
        assert!(!version_satisfies_requirement("21.0.0", "20"));
    }

    #[test]
    fn test_version_matches_segments() {
        assert!(version_matches_segments("0.2.0", "0.2"));
        assert!(version_matches_segments("0.2.1", "0.2"));
        assert!(!version_matches_segments("0.20.0", "0.2"));
        assert!(!version_matches_segments("10.0.0", "1"));
    }
}
