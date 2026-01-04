use anyhow::Result;
use serde::{Deserialize, Deserializer, Serialize};

/// Newtype for a SHA256 hash string (64 hex characters).
///
/// Provides compile-time distinction from other strings and optional runtime validation.
/// Primarily used for stored/indexed data where validation might have occurred earlier or been skipped.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct Sha256Hash(String);

impl Sha256Hash {
    /// Create a new Sha256Hash without validation (for index/deserialized data).
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// Create a validated Sha256Hash (64 hex characters).
    pub fn validated(s: &str) -> Result<Self, String> {
        if s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()) {
            Ok(Self(s.to_string()))
        } else {
            Err(format!(
                "Invalid SHA256 hash: expected 64 hex chars, got '{s}'"
            ))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Sha256Hash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AsRef<str> for Sha256Hash {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl From<String> for Sha256Hash {
    fn from(s: String) -> Self {
        Self::new(s)
    }
}

impl From<&str> for Sha256Hash {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

/// A validated SHA256 digest (64 hex characters)
///
/// This newtype ensures that all digests in the system are validated at deserialization time,
/// preventing invalid hex strings from propagating through the codebase.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct Sha256Digest(String);

impl Sha256Digest {
    /// Create a new Sha256Digest, validating the input
    ///
    /// Accepts strings with or without "sha256:" prefix.
    /// Returns an error if the digest is not exactly 64 hex characters.
    pub fn new(s: impl Into<String>) -> Result<Self> {
        let s = s.into();
        let hex = s.strip_prefix("sha256:").unwrap_or(&s);

        // Validate: exactly 64 hex chars
        if hex.len() != 64 {
            anyhow::bail!(
                "Invalid SHA256 digest: expected 64 hex characters, got {} in '{s}'",
                hex.len(),
            );
        }

        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            anyhow::bail!("Invalid SHA256 digest: contains non-hex characters in '{s}'");
        }

        Ok(Self(hex.to_lowercase()))
    }

    /// Get the digest as a string slice
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for Sha256Digest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::new(s).map_err(serde::de::Error::custom)
    }
}

impl std::fmt::Display for Sha256Digest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AsRef<str> for Sha256Digest {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

// Implement conversion from stricter digest to looser hash
impl From<Sha256Digest> for Sha256Hash {
    fn from(digest: Sha256Digest) -> Self {
        Sha256Hash::new(digest.as_str())
    }
}
