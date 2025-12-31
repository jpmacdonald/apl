//! Domain-specific errors for package operations

use crate::core::index::IndexError;
use crate::io::download::DownloadError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum InstallError {
    #[error("Failed to resolve dependencies: {0}")]
    Resolution(#[from] IndexError),

    #[error("Download failed: {0}")]
    Download(#[from] DownloadError),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Database lock poisoned: {0}")]
    Lock(String),

    #[error("Validation failed: {0}")]
    Validation(String),

    #[error("Build/Install script failed: {0}")]
    Script(String),

    #[error("{0}")]
    Other(String),
}

impl From<anyhow::Error> for InstallError {
    fn from(err: anyhow::Error) -> Self {
        Self::Other(err.to_string())
    }
}
