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
/// Versions are stored as strings to support arbitrary version formats
/// (e.g., `1.2.3`, `2024.01.01`, `nightly`). Comparison and ordering
/// are performed using semantic version parsing where applicable.
///
/// # Example
///
/// ```
/// use apl::types::Version;
///
/// let version = Version::new("1.7.1");
/// assert_eq!(version.as_str(), "1.7.1");
/// ```
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct Version(String);

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
