//! Public Output API - Clean interface for commands
//!
//! This module provides the main API that commands use to interact with the UI.
//! All operations are sent as events to the UI actor for sequential processing.
//!
//! # Example
//!
//! ```no_run
//! use apl::ui::Output;
//!
//! let output = Output::new();
//!
//! // Prepare for installs
//! output.prepare_pipeline(&[
//!     ("ripgrep".to_string(), Some("14.1.0".to_string())),
//! ]);
//!
//! // Show progress
//! output.downloading("ripgrep", "14.1.0", 1024, 4096);
//! output.installing("ripgrep", "14.1.0");
//! output.done("ripgrep", "14.1.0", "installed", Some(4096));
//!
//! // Summary
//! output.summary(1, "installed", 2.5);
//! ```

use super::actor::{UiActor, UiEvent};
use std::sync::{OnceLock, mpsc};

/// Global singleton actor instance
static UI_ACTOR: OnceLock<mpsc::Sender<UiEvent>> = OnceLock::new();

/// Initialize the global UI actor (called once)
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

/// Main output interface for UI operations
///
/// This is a lightweight handle that sends events to the UI actor thread.
/// It's cheap to clone and can be safely shared across threads.
#[derive(Clone)]
pub struct Output {
    sender: mpsc::Sender<UiEvent>,
}

impl Output {
    /// Create a new output handle
    ///
    /// This uses a global singleton actor, so multiple Output instances
    /// share the same rendering thread.
    pub fn new() -> Self {
        Self {
            sender: get_actor_sender(),
        }
    }

    /// Prepare table for a pipeline of packages
    ///
    /// This prints the header and reserves visual space for all packages.
    /// Call this before starting parallel downloads.
    pub fn prepare_pipeline(&self, packages: &[(String, Option<String>)]) {
        let _ = self.sender.send(UiEvent::PreparePipeline {
            items: packages.to_vec(),
        });
    }

    /// Print a simple section header
    pub fn section(&self, title: &str) {
        let _ = self.sender.send(UiEvent::PrintHeader {
            title: title.to_string(),
        });
    }

    /// Update package to downloading state
    pub fn downloading(&self, name: &str, version: &str, current: u64, total: u64) {
        let _ = self.sender.send(UiEvent::Downloading {
            name: name.to_string(),
            version: version.to_string(),
            current,
            total,
        });
    }

    /// Update package to installing state
    pub fn installing(&self, name: &str, version: &str) {
        let _ = self.sender.send(UiEvent::Installing {
            name: name.to_string(),
            version: version.to_string(),
        });
    }

    /// Update package to removing state
    pub fn removing(&self, name: &str, version: &str) {
        let _ = self.sender.send(UiEvent::Removing {
            name: name.to_string(),
            version: version.to_string(),
        });
    }

    /// Mark package as successfully completed
    pub fn done(&self, name: &str, version: &str, detail: &str, size: Option<u64>) {
        let _ = self.sender.send(UiEvent::Done {
            name: name.to_string(),
            version: version.to_string(),
            detail: detail.to_string(),
            size,
        });
    }

    /// Mark package as failed
    pub fn failed(&self, name: &str, version: &str, reason: &str) {
        let _ = self.sender.send(UiEvent::Failed {
            name: name.to_string(),
            version: version.to_string(),
            reason: reason.to_string(),
        });
    }

    /// Print info message
    pub fn info(&self, msg: &str) {
        let _ = self.sender.send(UiEvent::Info(msg.to_string()));
    }

    /// Print success message
    pub fn success(&self, msg: &str) {
        let _ = self.sender.send(UiEvent::Success(msg.to_string()));
    }

    /// Print warning message
    pub fn warning(&self, msg: &str) {
        let _ = self.sender.send(UiEvent::Warning(msg.to_string()));
    }

    /// Print error message
    pub fn error(&self, msg: &str) {
        let _ = self.sender.send(UiEvent::Error(msg.to_string()));
    }

    /// Print summary with timing info
    pub fn summary(&self, count: usize, action: &str, elapsed_secs: f64) {
        let _ = self.sender.send(UiEvent::Summary {
            count,
            action: action.to_string(),
            elapsed_secs,
        });
    }

    /// Print plain summary without timing
    pub fn summary_plain(&self, count: usize, status: &str) {
        let msg = format!(
            "{} package{} {}",
            count,
            if count == 1 { "" } else { "s" },
            status
        );
        self.success(&msg);
    }

    /// Print success summary (convenience alias)
    pub fn success_summary(&self, msg: &str) {
        self.success(msg);
    }

    /// Print error summary (convenience alias)
    pub fn error_summary(&self, msg: &str) {
        self.error(msg);
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
        // Output will be silently sent to actor
    }

    #[test]
    fn test_output_clone() {
        let output = Output::new();
        let output2 = output.clone();

        output.info("from original");
        output2.info("from clone");
    }
}
