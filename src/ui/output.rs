//! Unified UI output interface.
//!
//! This module provides the main API that commands use to interact with the UI.
//! All operations are sent as events to the UI actor for sequential processing.

use super::actor::{UiActor, UiEvent};
use crate::types::{PackageName, Version};
use std::sync::{OnceLock, mpsc};

/// Singleton instance of the UI actor channel.
static UI_ACTOR: OnceLock<mpsc::Sender<UiEvent>> = OnceLock::new();

/// Lazily initializes the UI actor and returns a sender handle.
fn get_actor_sender() -> mpsc::Sender<UiEvent> {
    UI_ACTOR
        .get_or_init(|| {
            let actor = UiActor::spawn();
            let sender = actor.sender();

            // Keep actor alive for program duration
            std::mem::forget(actor);

            sender
        })
        .clone()
}

/// A cloneable handle for sending high-level UI events to the terminal actor.
#[derive(Clone)]
pub struct Output {
    sender: mpsc::Sender<UiEvent>,
}

impl Output {
    /// Create a new output handle.
    pub fn new() -> Self {
        Self {
            sender: get_actor_sender(),
        }
    }

    /// Prepare table for a pipeline of packages.
    pub fn prepare_pipeline(&self, packages: &[(PackageName, Option<Version>)]) {
        let _ = self.sender.send(UiEvent::PreparePipeline {
            items: packages.to_vec(),
        });
    }

    /// Prints a visual section header for an operation phase.
    pub fn section(&self, title: &str) {
        let _ = self.sender.send(UiEvent::PrintHeader {
            title: title.to_string(),
        });
    }

    /// Reports progress for a file download.
    pub fn downloading(&self, name: &PackageName, version: &Version, current: u64, total: u64) {
        let _ = self.sender.send(UiEvent::Downloading {
            name: name.clone(),
            version: version.clone(),
            current,
            total,
        });
    }

    /// Transitions a package display to the 'installing' state.
    pub fn installing(&self, name: &PackageName, version: &Version) {
        let _ = self.sender.send(UiEvent::Installing {
            name: name.clone(),
            version: version.clone(),
        });
    }

    /// Transitions a package display to the 'removing' state.
    pub fn removing(&self, name: &PackageName, version: &Version) {
        let _ = self.sender.send(UiEvent::Removing {
            name: name.clone(),
            version: version.clone(),
        });
    }

    /// Signals completion of a package operation.
    pub fn done(&self, name: &PackageName, version: &Version, detail: &str, size: Option<u64>) {
        let _ = self.sender.send(UiEvent::Done {
            name: name.clone(),
            version: version.clone(),
            detail: detail.to_string(),
            size,
        });
    }

    /// Marks a package operation as failed with a visible reason.
    pub fn failed(&self, name: &PackageName, version: &Version, reason: &str) {
        let _ = self.sender.send(UiEvent::Failed {
            name: name.clone(),
            version: version.clone(),
            reason: reason.to_string(),
        });
    }

    /// Prints an informational message to the console.
    pub fn info(&self, msg: &str) {
        let _ = self.sender.send(UiEvent::Info(msg.to_string()));
    }

    /// Prints a success message to the console.
    pub fn success(&self, msg: &str) {
        let _ = self.sender.send(UiEvent::Success(msg.to_string()));
    }

    /// Prints a warning message to the console.
    pub fn warning(&self, msg: &str) {
        let _ = self.sender.send(UiEvent::Warning(msg.to_string()));
    }

    /// Prints an error message to the console.
    pub fn error(&self, msg: &str) {
        let _ = self.sender.send(UiEvent::Error(msg.to_string()));
    }

    /// Prints a summary of operations including the total elapsed time.
    pub fn summary(&self, count: usize, action: &str, elapsed_secs: f64) {
        let _ = self.sender.send(UiEvent::Summary {
            count,
            action: action.to_string(),
            elapsed_secs,
        });
    }

    /// Displays a summary of operations with item count and status.
    pub fn summary_plain(&self, count: usize, status: &str) {
        let msg = format!(
            "{} package{} {}",
            count,
            if count == 1 { "" } else { "s" },
            status
        );
        self.success(&msg);
    }

    /// Convenience for a success summary.
    pub fn success_summary(&self, msg: &str) {
        self.success(msg);
    }

    /// Convenience for an error summary.
    pub fn error_summary(&self, msg: &str) {
        self.error(msg);
    }

    /// Block until all pending UI events are processed.
    pub fn wait(&self) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let _ = self.sender.send(UiEvent::Sync(tx));

        // Block effectively without spinning CPU
        let _ = rx.blocking_recv();
    }

    /// Async version of wait.
    pub async fn wait_async(&self) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let _ = self.sender.send(UiEvent::Sync(tx));

        let _ = rx.await;
    }
}

impl super::reporter::Reporter for Output {
    fn prepare_pipeline(&self, packages: &[(PackageName, Option<Version>)]) {
        self.prepare_pipeline(packages);
    }

    fn section(&self, title: &str) {
        self.section(title);
    }

    fn downloading(&self, name: &PackageName, version: &Version, current: u64, total: u64) {
        self.downloading(name, version, current, total);
    }

    fn installing(&self, name: &PackageName, version: &Version) {
        self.installing(name, version);
    }

    fn removing(&self, name: &PackageName, version: &Version) {
        self.removing(name, version);
    }

    fn done(&self, name: &PackageName, version: &Version, detail: &str, size: Option<u64>) {
        self.done(name, version, detail, size);
    }

    fn failed(&self, name: &PackageName, version: &Version, reason: &str) {
        self.failed(name, version, reason);
    }

    fn info(&self, msg: &str) {
        self.info(msg);
    }

    fn success(&self, msg: &str) {
        self.success(msg);
    }

    fn warning(&self, msg: &str) {
        self.warning(msg);
    }

    fn error(&self, msg: &str) {
        self.error(msg);
    }

    fn summary(&self, count: usize, action: &str, elapsed_secs: f64) {
        self.summary(count, action, elapsed_secs);
    }

    fn summary_plain(&self, count: usize, status: &str) {
        self.summary_plain(count, status);
    }
}

impl Default for Output {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_output_creation() {
        let output = Output::new();
        output.info("test");
    }

    #[test]
    fn test_output_clone() {
        let output = Output::new();
        let output2 = output.clone();

        output.info("from original");
        output2.info("from clone");
    }
}
