//! Reporter trait for dependency injection
//!
//! This trait allows core logic to report progress and status without
//! being coupled to a specific TUI or GUI implementation.

use crate::types::{PackageName, Version};

pub trait Reporter: Send + Sync {
    /// Reserve space for a set of packages in the output display.
    fn prepare_pipeline(&self, packages: &[(PackageName, Option<Version>)]);

    /// Indicates a new section or phase has started (e.g. "Fetching", "Installing").
    fn section(&self, title: &str);

    /// Updates the progress of a download.
    fn downloading(&self, name: &PackageName, version: &Version, current: u64, total: u64);

    /// Updates the state of a package to 'installing'.
    fn installing(&self, name: &PackageName, version: &Version);

    /// Updates the state of a package to 'removing'.
    fn removing(&self, name: &PackageName, version: &Version);

    /// Marks a package operation as successfully completed.
    fn done(&self, name: &PackageName, version: &Version, detail: &str, size: Option<u64>);

    /// Marks a package operation as failed with a specific reason.
    fn failed(&self, name: &PackageName, version: &Version, reason: &str);

    /// Log an informational message.
    fn info(&self, msg: &str);

    /// Log a success message.
    fn success(&self, msg: &str);

    /// Log a warning message.
    fn warning(&self, msg: &str);

    /// Log an error message.
    fn error(&self, msg: &str);

    /// Display a final summary of multiple operations.
    fn summary(&self, count: usize, action: &str, elapsed_secs: f64);

    /// Display a final summary without timing information.
    fn summary_plain(&self, count: usize, status: &str);
}
