/// A normalized package name.
///
/// Package names are automatically lowercased to ensure consistent lookups
/// and comparisons. This prevents issues with case-sensitive package names
/// like `JQ` vs `jq`.
///
/// # Example
///
/// ```
/// use apl::types::PackageName;
///
/// let name = PackageName::new("JQ");
/// assert_eq!(name.as_str(), "jq");
/// ```
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct PackageName(String);

impl PackageName {
    /// Create a new package name, automatically normalizing to lowercase.
    pub fn new(name: &str) -> Self {
        Self(name.to_lowercase())
    }

    /// Get the normalized package name as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<std::ffi::OsStr> for PackageName {
    fn as_ref(&self) -> &std::ffi::OsStr {
        self.0.as_ref()
    }
}

impl AsRef<std::path::Path> for PackageName {
    fn as_ref(&self) -> &std::path::Path {
        std::path::Path::new(&self.0)
    }
}

impl std::fmt::Display for PackageName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::ops::Deref for PackageName {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<str> for PackageName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl PartialEq<str> for PackageName {
    fn eq(&self, other: &str) -> bool {
        self.0 == other.to_lowercase()
    }
}

impl PartialEq<&str> for PackageName {
    fn eq(&self, other: &&str) -> bool {
        self.0 == other.to_lowercase()
    }
}

impl PartialEq<String> for PackageName {
    fn eq(&self, other: &String) -> bool {
        self.0 == other.to_lowercase()
    }
}

impl Borrow<str> for PackageName {
    fn borrow(&self) -> &str {
        &self.0
    }
}

use std::borrow::Borrow;

impl From<&str> for PackageName {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for PackageName {
    fn from(s: String) -> Self {
        Self::new(&s)
    }
}

/// A semantic version string.
///
/// Versions are stored as strings but compared using semantic versioning rules.
/// All versions are expected to be normalized to semver format by `auto_parse_version`
/// before storage. If a version fails to parse as semver, a debug assertion fires
/// to catch normalization bugs early.
///
/// # Example
///
/// ```
/// use apl::types::Version;
///
/// let v1 = Version::new("0.9.1");
/// let v2 = Version::new("0.12.0");
/// assert!(v2 > v1); // Semver comparison, not string!
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Version(String);

impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match (
            semver::Version::parse(&self.0),
            semver::Version::parse(&other.0),
        ) {
            (Ok(a), Ok(b)) => a.cmp(&b),
            // If either fails to parse, it's a bug in normalization - catch in debug
            (Ok(_), Err(_)) => std::cmp::Ordering::Less,
            (Err(_), Ok(_)) => std::cmp::Ordering::Greater,
            (Err(_), Err(_)) => {
                debug_assert!(
                    false,
                    "Non-semver versions in comparison: '{}' vs '{}' - normalization bug",
                    self.0, other.0
                );
                self.0.cmp(&other.0)
            }
        }
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Version {
    /// Create a new version from a string.
    pub fn new(v: &str) -> Self {
        Self(v.to_string())
    }

    /// Get the version string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::ops::Deref for Version {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<str> for Version {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl AsRef<std::ffi::OsStr> for Version {
    fn as_ref(&self) -> &std::ffi::OsStr {
        self.0.as_ref()
    }
}

impl AsRef<std::path::Path> for Version {
    fn as_ref(&self) -> &std::path::Path {
        std::path::Path::new(&self.0)
    }
}

impl From<&str> for Version {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl From<String> for Version {
    fn from(s: String) -> Self {
        Self::new(&s)
    }
}

impl PartialEq<str> for Version {
    fn eq(&self, other: &str) -> bool {
        self.0 == other
    }
}

impl PartialEq<&str> for Version {
    fn eq(&self, other: &&str) -> bool {
        self.0 == *other
    }
}

impl PartialEq<String> for Version {
    fn eq(&self, other: &String) -> bool {
        self.0 == *other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_semver_ordering() {
        // This is the bug we fixed: 0.12.0 > 0.9.1 in semver, but not alphabetically
        let v1 = Version::new("0.9.1");
        let v2 = Version::new("0.12.0");
        assert!(v2 > v1, "0.12.0 should be greater than 0.9.1");

        // More ordering tests
        let versions = vec![
            Version::new("0.8.0"),
            Version::new("1.0.0"),
            Version::new("0.10.0"),
            Version::new("0.9.1"),
            Version::new("0.12.0"),
        ];
        let mut sorted = versions.clone();
        sorted.sort();

        let expected: Vec<&str> = vec!["0.8.0", "0.9.1", "0.10.0", "0.12.0", "1.0.0"];
        let actual: Vec<&str> = sorted.iter().map(|v| v.as_str()).collect();
        assert_eq!(actual, expected);
    }
}
