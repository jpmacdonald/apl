use serde::{Deserialize, Serialize};

/// Represents an artifact in the APL index (e.g., index.json).
/// This structure is shared between the Engine (producer) and Core (consumer).
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Artifact {
    /// Name of the package (e.g., "terraform")
    pub name: String,
    
    /// Version string (e.g., "1.5.0")
    pub version: String,
    
    /// Architecture this artifact supports (e.g., "x86_64-apple-darwin")
    pub arch: String,
    
    /// Download URL (original vendor URL or R2 mirror)
    pub url: String,
    
    /// SHA256 Checksum
    pub sha256: String,
}

#[derive(thiserror::Error, Debug)]
pub enum ArtifactError {
    #[error("Invalid SHA256 length: expected 64 chars, got {0}")]
    InvalidSha256Length(usize),
    
    #[error("Empty field: {0}")]
    EmptyField(String),

    #[error("Invalid URL: {0}")]
    InvalidUrl(String),
}

impl Artifact {
    /// Validates the artifact's integrity.
    pub fn validate(&self) -> Result<(), ArtifactError> {
        if self.name.is_empty() {
             return Err(ArtifactError::EmptyField("name".to_string()));
        }
        if self.version.is_empty() {
             return Err(ArtifactError::EmptyField("version".to_string()));
        }
        if self.url.is_empty() {
             return Err(ArtifactError::EmptyField("url".to_string()));
        }
        if !self.url.starts_with("http") {
            return Err(ArtifactError::InvalidUrl("Must start with http(s)".to_string()));
        }
        
        // Strict SHA256 validation
        if self.sha256.len() != 64 {
             return Err(ArtifactError::InvalidSha256Length(self.sha256.len()));
        }
        // Could also check hex chars, but length is a good first step.
        
        Ok(())
    }
}


/// Declarative configuration for a Port, parsed from `port.toml`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "strategy", rename_all = "lowercase")]
pub enum PortConfig {
    /// Generic JSON feed strategy (e.g., HashiCorp, Go, Node)
    #[serde(rename = "hashicorp")]
    HashiCorp {
        product: String,
    },
    
    #[serde(rename = "golang")]
    Golang,

    #[serde(rename = "node")]
    Node,

    #[serde(rename = "github")]
    GitHub {
        owner: String,
        repo: String,
    },
    
    #[serde(rename = "custom")]
    Custom, // Fallback for complex logic

    #[serde(rename = "aws")]
    Aws,

    #[serde(rename = "python")]
    Python,

    #[serde(rename = "ruby")]
    Ruby,
}

/// Top-level structure for `port.toml`
#[derive(Debug, Serialize, Deserialize)]
pub struct PortManifest {
    pub package: PackageMeta,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PackageMeta {
    pub name: String,
    #[serde(flatten)]
    pub config: PortConfig,
    
    /// Optional URL override if strategy needs a base URL
    pub url: Option<String>,
}
