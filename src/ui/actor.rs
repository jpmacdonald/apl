//! UI Actor - Single-threaded event processing
//!
//! This module implements the actor pattern for UI rendering.
//! All UI operations are channeled through a single thread to prevent
//! race conditions and output corruption.
//!
//! # Benefits
//!
//! - **No contention**: Workers never wait for locks
//! - **Crash safety**: UI stays alive even if workers panic
//! - **Sequential rendering**: Guaranteed ordering of updates
//! - **Testability**: Can record/replay events

use super::buffer::OutputBuffer;
use super::table::{PackageState, TableRenderer};
use super::theme::Theme;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// Events that can be sent to the UI actor
#[derive(Debug, Clone)]
pub enum UiEvent {
    /// Prepare the table for a pipeline of packages
    PreparePipeline {
        items: Vec<(String, Option<String>)>,
    },
    /// Print a simple header section
    PrintHeader { title: String },
    /// Update package state to downloading
    Downloading {
        name: String,
        version: String,
        current: u64,
        total: u64,
    },
    /// Update package state to installing
    Installing { name: String, version: String },
    /// Update package state to removing
    Removing { name: String, version: String },
    /// Mark package as successfully done
    Done {
        name: String,
        version: String,
        detail: String,
        size: Option<u64>,
    },
    /// Mark package as failed
    Failed {
        name: String,
        version: String,
        reason: String,
    },
    /// Print info message
    Info(String),
    /// Print success footer
    Success(String),
    /// Print warning footer
    Warning(String),
    /// Print error footer
    Error(String),
    /// Print summary with timing
    Summary {
        count: usize,
        action: String,
        elapsed_secs: f64,
    },
    /// Shutdown the actor
    Shutdown,
}

/// Handle to the UI actor thread
pub struct UiActor {
    sender: mpsc::Sender<UiEvent>,
    _handle: thread::JoinHandle<()>,
}

impl UiActor {
    /// Spawn a new UI actor thread
    pub fn spawn() -> Self {
        let (sender, receiver) = mpsc::channel();

        let handle = thread::spawn(move || {
            run_event_loop(receiver);
        });

        Self {
            sender,
            _handle: handle,
        }
    }

    /// Get a cloneable sender for this actor
    pub fn sender(&self) -> mpsc::Sender<UiEvent> {
        self.sender.clone()
    }
}

impl Drop for UiActor {
    fn drop(&mut self) {
        // Send shutdown signal (ignore errors if already shut down)
        let _ = self.sender.send(UiEvent::Shutdown);
    }
}

/// Main event loop for the UI actor
///
/// This runs in a dedicated thread and processes all UI events sequentially.
fn run_event_loop(receiver: mpsc::Receiver<UiEvent>) {
    let mut buffer = OutputBuffer::default();
    let theme = Theme::default();
    let mut table = TableRenderer::new(theme.clone());

    loop {
        // Use timeout to drive animations (100ms = 10 FPS)
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(UiEvent::PreparePipeline { items }) => {
                table.prepare_pipeline(&mut buffer, &items);
            }
            Ok(UiEvent::PrintHeader { title }) => {
                println!();
                println!("{} {}", title, "─".repeat(40));
                buffer.flush();
            }
            Ok(UiEvent::Downloading {
                name,
                version,
                current,
                total,
            }) => {
                table.update_package(
                    &name,
                    Some(&version),
                    PackageState::Downloading { current, total },
                    Some(total),
                );
                table.render_all(&mut buffer);
            }
            Ok(UiEvent::Installing { name, version }) => {
                table.update_package(&name, Some(&version), PackageState::Installing, None);
                table.render_all(&mut buffer);
            }
            Ok(UiEvent::Removing { name, version }) => {
                table.update_package(&name, Some(&version), PackageState::Removing, None);
                table.render_all(&mut buffer);
            }
            Ok(UiEvent::Done {
                name,
                version,
                detail,
                size,
            }) => {
                table.update_package(&name, Some(&version), PackageState::Done { detail }, size);
                table.render_all(&mut buffer);
            }
            Ok(UiEvent::Failed {
                name,
                version,
                reason,
            }) => {
                table.update_package(&name, Some(&version), PackageState::Failed { reason }, None);
                table.render_all(&mut buffer);
            }
            Ok(UiEvent::Info(msg)) => {
                println!("  {} {}", theme.icons.pending, msg);
                buffer.flush();
            }
            Ok(UiEvent::Success(msg)) => {
                table.print_footer(&mut buffer, &msg, true);
            }
            Ok(UiEvent::Warning(msg)) => {
                table.print_footer(&mut buffer, &msg, false);
            }
            Ok(UiEvent::Error(msg)) => {
                println!("{} {}", theme.icons.error, msg);
                buffer.flush();
            }
            Ok(UiEvent::Summary {
                count,
                action,
                elapsed_secs,
            }) => {
                let msg = format!(
                    "{} package{} {} in {:.1}s",
                    count,
                    if count == 1 { "" } else { "s" },
                    action,
                    elapsed_secs
                );
                table.print_footer(&mut buffer, &msg, true);
            }
            Ok(UiEvent::Shutdown) => {
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Animate active rows
                table.render_active(&mut buffer);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ui_event_variants() {
        let event = UiEvent::Info("test".to_string());
        assert!(matches!(event, UiEvent::Info(_)));

        let event2 = UiEvent::Downloading {
            name: "pkg".to_string(),
            version: "1.0.0".to_string(),
            current: 100,
            total: 200,
        };
        assert!(matches!(event2, UiEvent::Downloading { .. }));
    }

    #[test]
    fn test_actor_spawn() {
        let actor = UiActor::spawn();
        let sender = actor.sender();

        // Send a test event
        sender.send(UiEvent::Info("test".to_string())).unwrap();

        // Actor will shutdown when dropped
        drop(actor);
    }
}
