//! UI Actor - Message-passing architecture for concurrent UI updates
//!
//! This module implements an actor-based approach to UI rendering, eliminating
//! mutex contention and preventing deadlocks during parallel operations.

use crossterm::style::Color;
use std::sync::mpsc;
use std::thread;

/// Events that can be sent to the UI actor
#[derive(Debug, Clone)]
pub enum UiEvent {
    /// Register a new package in the UI
    AddPackage { name: String, version: String },
    /// Update download progress for a package
    Progress {
        name: String,
        bytes_downloaded: u64,
        total_bytes: u64,
    },
    /// Mark package as "installing" (extracting/linking)
    SetInstalling { name: String, version: String },
    /// Mark package as complete
    Done {
        name: String,
        version: String,
        status: String,
        size_bytes: Option<u64>,
    },
    /// Mark package as failed
    Fail {
        name: String,
        version: String,
        error: String,
    },
    /// Display a log message
    Log {
        message: String,
        color: Option<Color>,
    },
    /// Display summary footer
    Summary {
        count: usize,
        action: String,
        elapsed_secs: f64,
    },
    /// Display plain text summary (no timing)
    SummaryPlain { count: usize, status: String },
    /// Display info message
    Info(String),
    /// Display success message
    Success(String),
    /// Display warning message
    Warning(String),
    /// Display error message
    Error(String),
    /// Shutdown the UI actor
    Shutdown,
}

/// Handle to the UI actor thread
pub struct UiActor {
    sender: mpsc::Sender<UiEvent>,
    handle: Option<thread::JoinHandle<()>>,
}

impl UiActor {
    /// Spawn a new UI actor thread
    pub fn spawn() -> Self {
        let (sender, receiver) = mpsc::channel();

        let handle = thread::spawn(move || {
            run_ui_loop(receiver);
        });

        Self {
            sender,
            handle: Some(handle),
        }
    }

    /// Get a cloneable sender for this actor
    pub fn sender(&self) -> mpsc::Sender<UiEvent> {
        self.sender.clone()
    }

    /// Send an event to the UI actor
    pub fn send(&self, event: UiEvent) {
        // Ignore send errors (UI might have shut down)
        let _ = self.sender.send(event);
    }

    /// Shutdown the UI actor and wait for it to finish
    pub fn shutdown(mut self) {
        let _ = self.sender.send(UiEvent::Shutdown);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for UiActor {
    fn drop(&mut self) {
        // Send shutdown if not already sent
        let _ = self.sender.send(UiEvent::Shutdown);
        // Wait for thread to finish
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Main event loop for the UI actor
fn run_ui_loop(receiver: mpsc::Receiver<UiEvent>) {
    use crate::io::output::TableOutput;

    let mut output = TableOutput::new();

    while let Ok(event) = receiver.recv() {
        match event {
            UiEvent::AddPackage { name, version } => {
                output.add_package(&name, &version, 0);
            }
            UiEvent::Progress {
                name,
                bytes_downloaded,
                total_bytes: _,
            } => {
                output.update_progress(&name, bytes_downloaded);
            }
            UiEvent::SetInstalling { name, version: _ } => {
                output.set_installing(&name);
            }
            UiEvent::Done {
                name,
                version: _,
                status,
                size_bytes,
            } => {
                output.set_done(&name, &status, size_bytes);
            }
            UiEvent::Fail {
                name,
                version: _,
                error,
            } => {
                output.set_failed(&name, &error);
            }
            UiEvent::Shutdown => {
                break;
            }
            _ => {
                // Unsupported events - ignore for now
            }
        }
    }
}
