//! Domain-specific errors for package operations

use apl_core::io::download::DownloadError;
use apl_schema::index::IndexError;
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

    #[error("{context}: {message}")]
    Context {
        context: &'static str,
        message: String,
    },

    #[error("{0}")]
    Other(String),
}

impl InstallError {
    /// Create an error with context for better debugging.
    pub fn context(ctx: &'static str, msg: impl std::fmt::Display) -> Self {
        Self::Context {
            context: ctx,
            message: msg.to_string(),
        }
    }
}

impl From<anyhow::Error> for InstallError {
    fn from(err: anyhow::Error) -> Self {
        Self::Other(err.to_string())
    }
}
