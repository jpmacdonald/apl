//! UI Actor - Single-threaded event processing
//!
//! This module implements the actor pattern for UI rendering.
//! All UI operations are channeled through a single thread to prevent
//! race conditions and output corruption.
//!
/// # Implementation Note: The Actor Pattern in UI
///
/// We use the "Actor Model" here to bridge the gap between our parallel, async download tasks
/// and the strictly serial nature of terminal output (stdout).
///
/// 1. **Sender (Many)**: Download tasks `clone()` the sender and fire events (Downloading, Done, etc.)
///    asynchronously. They don't block waiting for the terminal.
///
/// 2. **Receiver (One)**: The `UiActor` thread owns the `receiver` and processes events one by one.
///    This guarantees that two threads never try to write to the console at the exact same time,
///    which would cause "tearing" or garbled lines.
///
/// 3. **State Management**: The Actor is the *exclusive* owner of the `TableRenderer` state.
///    Because only the actor thread touches the table, we don't need `Mutex<TableRenderer>`
///    or complex locking in the application logic.
use super::buffer::OutputBuffer;
use super::table::{PackageState, Severity, TableRenderer};
use super::theme::Theme;
use apl_schema::types::{PackageName, Version};
use crossterm::style::Stylize;
use std::fmt;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// Events that can be sent to the UI actor
#[derive(Debug)]
pub enum UiEvent {
    /// Prepare the table for a pipeline of packages
    PreparePipeline {
        items: Vec<(PackageName, Option<Version>, usize)>,
    },
    /// Print a simple header section
    PrintHeader { title: String },
    /// Live Phase: Print "Phase X: Title..." without newline and flush
    LivePhase { title: String },
    /// Live Phase Update: Append status and newline
    LivePhaseUpdate { status: String, success: bool },
    /// Update package state to downloading
    Downloading {
        name: PackageName,
        version: Version,
        current: u64,
        total: Option<u64>,
    },
    /// Update package state to extracting
    Extracting {
        name: PackageName,
        version: Version,
        current: u64,
        total: Option<u64>,
    },
    /// Update package state to installing
    Installing {
        name: PackageName,
        version: Version,
        current: Option<u64>,
        total: Option<u64>,
    },
    /// Update package state to removing
    Removing { name: PackageName, version: Version },
    /// Mark package as successfully done
    Done {
        name: PackageName,
        version: Version,
        detail: String,
        size: Option<u64>,
    },
    /// Mark package as failed
    Failed {
        name: PackageName,
        version: Version,
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
    /// Synchronize UI state (wait for all pending renders)
    Sync(tokio::sync::oneshot::Sender<()>),
    /// Shutdown the actor
    Shutdown,
}

/// Handle to the UI actor thread
pub struct UiActor {
    sender: mpsc::Sender<UiEvent>,
    _handle: thread::JoinHandle<()>,
}

impl fmt::Debug for UiActor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("UiActor").finish_non_exhaustive()
    }
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
// The receiver is intentionally moved into this thread for exclusive ownership
// in the actor pattern.
#[allow(clippy::needless_pass_by_value)]
fn run_event_loop(receiver: mpsc::Receiver<UiEvent>) {
    let mut buffer = OutputBuffer::default();
    let theme = Theme::default();
    let mut table = TableRenderer::new(theme.clone());

    loop {
        // Use timeout to drive animations (100ms = 10 FPS)
        let event = match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(e) => e,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                table.render_active(&mut buffer);
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };

        // Process the first event
        let mut shutdown = process_event(event, &mut table, &mut buffer, &theme);

        // Drain any other immediately available events to batch updates
        while let Ok(next_event) = receiver.try_recv() {
            if process_event(next_event, &mut table, &mut buffer, &theme) {
                shutdown = true;
            }
        }

        // Render once after batch update
        table.render_all(&mut buffer);

        if shutdown {
            break;
        }
    }
}

/// Process a single event and update state. Returns true if shutdown received.
fn process_event(
    event: UiEvent,
    table: &mut TableRenderer,
    buffer: &mut OutputBuffer,
    theme: &Theme,
) -> bool {
    match event {
        UiEvent::PreparePipeline { items } => {
            table.prepare_pipeline(buffer, &items);
        }
        UiEvent::PrintHeader { title } => {
            println!();
            println!("{}", title.bold());
            buffer.flush();
        }
        UiEvent::LivePhase { title } => {
            use std::io::Write;
            let padded = format!("{: <width$}", title, width = theme.layout.phase_padding);
            print!("{}", padded.dark_grey());
            let _ = std::io::stdout().flush();
        }
        UiEvent::LivePhaseUpdate { status, success } => {
            if success {
                println!("{}", status.green().bold());
            } else {
                println!("{}", status.red().bold());
            }
        }
        UiEvent::Downloading {
            name,
            version,
            current,
            total,
        } => {
            let total = total
                .filter(|&t| t > 0)
                .or_else(|| table.get_package_size(&name));
            table.update_package(
                &name,
                Some(&version),
                PackageState::Downloading { current, total },
                total,
            );
        }
        UiEvent::Extracting {
            name,
            version,
            current,
            total,
        } => {
            let total = total
                .filter(|&t| t > 0)
                .or_else(|| table.get_package_size(&name));
            table.update_package(
                &name,
                Some(&version),
                PackageState::Extracting { current, total },
                total,
            );
        }
        UiEvent::Installing {
            name,
            version,
            current,
            total,
        } => {
            let total = total
                .filter(|&t| t > 0)
                .or_else(|| table.get_package_size(&name));
            let state = if let Some(t) = total {
                PackageState::Installing {
                    current: current.filter(|&c| c > 0).unwrap_or(t),
                    total: Some(t),
                }
            } else {
                PackageState::Installing {
                    current: current.unwrap_or(0),
                    total: None,
                }
            };
            table.update_package(&name, Some(&version), state, total);
        }
        UiEvent::Removing { name, version } => {
            table.update_package(&name, Some(&version), PackageState::Removing, None);
        }
        UiEvent::Done {
            name,
            version,
            detail,
            size,
        } => {
            table.update_package(&name, Some(&version), PackageState::Done { detail }, size);
        }
        UiEvent::Failed {
            name,
            version,
            reason,
        } => {
            table.update_package(&name, Some(&version), PackageState::Failed { reason }, None);
        }
        UiEvent::Info(msg) => {
            println!("  {} {}", theme.icons.info, msg);
            buffer.flush();
        }
        UiEvent::Success(msg) => {
            table.print_footer(buffer, &msg, Severity::Success);
        }
        UiEvent::Warning(msg) => {
            table.print_footer(buffer, &msg, Severity::Warning);
        }
        UiEvent::Error(msg) => {
            table.print_footer(buffer, &msg, Severity::Error);
        }
        UiEvent::Summary {
            count,
            action,
            elapsed_secs,
        } => {
            let operation = action.to_uppercase();
            let msg = format!("{operation} COMPLETE {count}, elapsed {elapsed_secs:.1}s");
            table.print_plain(buffer, &msg);
        }
        UiEvent::Sync(tx) => {
            let _ = tx.send(());
        }
        UiEvent::Shutdown => return true,
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ui_event_variants() {
        let event = UiEvent::Info("test".to_string());
        assert!(matches!(event, UiEvent::Info(_)));

        let event2 = UiEvent::Downloading {
            name: PackageName::new("pkg"),
            version: Version::from("1.0.0"),
            current: 100,
            total: Some(200),
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
