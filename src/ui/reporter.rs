//! Reporter trait for dependency injection
//!
//! This trait allows core logic to report progress and status without
//! being coupled to a specific TUI or GUI implementation.

use crate::types::{PackageName, Version};

pub trait Reporter: Send + Sync {
    /// Prepare a live-updated phase (e.g. "Phase 1: Discovering sources...")
    fn live_phase(&self, title: &str);

    /// Update the current live phase with a status (e.g. "COMPLETE")
    fn live_phase_update(&self, status: &str, success: bool);

    /// Reserve space for a set of packages in the output display.
    fn prepare_pipeline(&self, packages: &[(PackageName, Option<Version>, usize)]);

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

impl<T: Reporter + ?Sized> Reporter for std::sync::Arc<T> {
    fn prepare_pipeline(&self, packages: &[(PackageName, Option<Version>, usize)]) {
        (**self).prepare_pipeline(packages)
    }
    fn section(&self, title: &str) {
        (**self).section(title)
    }
    fn downloading(&self, name: &PackageName, version: &Version, current: u64, total: u64) {
        (**self).downloading(name, version, current, total)
    }
    fn installing(&self, name: &PackageName, version: &Version) {
        (**self).installing(name, version)
    }
    fn removing(&self, name: &PackageName, version: &Version) {
        (**self).removing(name, version)
    }
    fn done(&self, name: &PackageName, version: &Version, detail: &str, size: Option<u64>) {
        (**self).done(name, version, detail, size)
    }
    fn failed(&self, name: &PackageName, version: &Version, reason: &str) {
        (**self).failed(name, version, reason)
    }
    fn info(&self, msg: &str) {
        (**self).info(msg)
    }
    fn success(&self, msg: &str) {
        (**self).success(msg)
    }
    fn warning(&self, msg: &str) {
        (**self).warning(msg)
    }
    fn error(&self, msg: &str) {
        (**self).error(msg)
    }
    fn summary(&self, count: usize, action: &str, elapsed_secs: f64) {
        (**self).summary(count, action, elapsed_secs)
    }
    fn summary_plain(&self, count: usize, status: &str) {
        (**self).summary_plain(count, status)
    }

    fn live_phase(&self, title: &str) {
        (**self).live_phase(title)
    }

    fn live_phase_update(&self, status: &str, success: bool) {
        (**self).live_phase_update(status, success)
    }
}

/// A no-op reporter for silent operations (e.g., verification, testing).
#[derive(Clone, Copy)]
pub struct NullReporter;

impl Reporter for NullReporter {
    fn live_phase(&self, _: &str) {}
    fn live_phase_update(&self, _: &str, _: bool) {}
    fn prepare_pipeline(&self, _: &[(PackageName, Option<Version>, usize)]) {}
    fn section(&self, _: &str) {}
    fn downloading(&self, _: &PackageName, _: &Version, _: u64, _: u64) {}
    fn installing(&self, _: &PackageName, _: &Version) {}
    fn removing(&self, _: &PackageName, _: &Version) {}
    fn done(&self, _: &PackageName, _: &Version, _: &str, _: Option<u64>) {}
    fn failed(&self, _: &PackageName, _: &Version, _: &str) {}
    fn info(&self, _: &str) {}
    fn success(&self, _: &str) {}
    fn warning(&self, _: &str) {}
    fn error(&self, _: &str) {}
    fn summary(&self, _: usize, _: &str, _: f64) {}
    fn summary_plain(&self, _: usize, _: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_reporter_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<NullReporter>();
    }

    #[test]
    fn null_reporter_implements_all_methods() {
        let reporter = NullReporter;
        let name = PackageName::from("test");
        let version = Version::from("1.0.0");

        // All methods should be no-ops (no panics)
        reporter.prepare_pipeline(&[(name.clone(), Some(version.clone()), 0)]);
        reporter.section("test");
        reporter.downloading(&name, &version, 0, 100);
        reporter.installing(&name, &version);
        reporter.removing(&name, &version);
        reporter.done(&name, &version, "done", Some(1024));
        reporter.failed(&name, &version, "error");
        reporter.info("info");
        reporter.success("success");
        reporter.warning("warning");
        reporter.error("error");
        reporter.summary(1, "installed", 1.0);
        reporter.summary_plain(1, "installed");
    }
}
