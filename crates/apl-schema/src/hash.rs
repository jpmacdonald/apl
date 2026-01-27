use anyhow::Result;
use serde::{Deserialize, Deserializer, Serialize};

/// Newtype for a SHA256 hash string (64 hex characters).
///
/// Provides compile-time distinction from other strings and optional runtime validation.
/// Primarily used for stored/indexed data where validation might have occurred earlier or been skipped.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub struct Sha256Hash(String);

impl Sha256Hash {
    /// Create a new `Sha256Hash` without validation (for index/deserialized data).
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// Create a validated `Sha256Hash` (64 hex characters).
    ///
    /// # Errors
    ///
    /// Returns an error string if `s` is not exactly 64 ASCII hex characters.
    pub fn validated(s: &str) -> Result<Self, String> {
        if s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()) {
            Ok(Self(s.to_string()))
        } else {
            Err(format!(
                "Invalid SHA256 hash: expected 64 hex chars, got '{s}'"
            ))
        }
    }

    /// Return the inner hex string as a string slice.
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
    /// Create a new `Sha256Digest`, validating the input.
    ///
    /// Accepts strings with or without a `sha256:` prefix.
    ///
    /// # Errors
    ///
    /// Returns an error if the hex portion is not exactly 64 ASCII hex characters.
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

/// BLAKE3 hash for fast internal operations (CAS keys, dedup).
///
/// BLAKE3 is ~7x faster than SHA256 on modern CPUs. Used for internal
/// content-addressable storage, while SHA256 is used for upstream verification.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Blake3Hash(String);

impl Blake3Hash {
    /// Create a new `Blake3Hash` from a raw hex string (64 hex chars).
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }

    /// Compute BLAKE3 hash of data.
    pub fn compute(data: &[u8]) -> Self {
        let hash = blake3::hash(data);
        Self(hash.to_hex().to_string())
    }

    /// Compute BLAKE3 hash of a file by reading it entirely into memory.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the file cannot be read.
    pub fn compute_file(path: &std::path::Path) -> std::io::Result<Self> {
        let data = std::fs::read(path)?;
        Ok(Self::compute(&data))
    }

    /// Return the inner hex string as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Blake3Hash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AsRef<str> for Blake3Hash {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blake3_compute_works() {
        let hash = Blake3Hash::compute(b"hello world");
        assert_eq!(hash.as_str().len(), 64); // 32 bytes = 64 hex chars
    }

    #[test]
    fn blake3_deterministic() {
        let h1 = Blake3Hash::compute(b"test data");
        let h2 = Blake3Hash::compute(b"test data");
        assert_eq!(h1, h2);
    }

    #[test]
    fn blake3_different_inputs_different_hashes() {
        let h1 = Blake3Hash::compute(b"input 1");
        let h2 = Blake3Hash::compute(b"input 2");
        assert_ne!(h1, h2);
    }
}
