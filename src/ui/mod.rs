//! UI Module - Clean separation of terminal output concerns
//!
//! This module provides a well-structured UI system for APL's terminal output.
//! All rendering logic is isolated here, making it easy to test, modify, and reason about.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────┐
//! │   Commands  │  (install, remove, list, etc.)
//! └──────┬──────┘
//!        │ uses
//!        ▼
//! ┌─────────────┐
//! │   Output    │  Public API for commands
//! └──────┬──────┘
//!        │ sends events
//!        ▼
//! ┌─────────────┐
//! │    Actor    │  Single-threaded event loop
//! └──────┬──────┘
//!        │ renders
//!        ▼
//! ┌─────────────┐
//! │  Renderer   │  Actual terminal output (Table, Buffer, etc.)
//! └──────┬──────┘
//!        │ styles with
//!        ▼
//! ┌─────────────┐
//! │    Theme    │  Colors, icons, constants
//! └─────────────┘
//! ```
//!
//! # Modules
//!
//! - [`theme`] - Colors, icons, and design constants
//! - [`buffer`] - Output buffering for atomic renders
//! - [`table`] - Table rendering for install/remove/list commands
//! - [`progress`] - Progress indicators and spinners
//! - [`actor`] - Message-passing event loop
//! - [`output`] - Public API for commands to use
//! - [`list`] - List formatting for installed packages
//!
//! # Example
//!
//! ```no_run
//! use apl::ui::Output;
//!
//! let output = Output::new();
//!
//! // Prepare for parallel downloads
//! output.prepare_pipeline(&[
//!     ("ripgrep".to_string(), Some("14.1.0".to_string())),
//!     ("fd".to_string(), Some("10.2.0".to_string())),
//! ]);
//!
//! // Update progress
//! output.downloading("ripgrep", "14.1.0", 1024, 4096);
//! output.installing("ripgrep", "14.1.0");
//! output.done("ripgrep", "14.1.0", "installed", None);
//!
//! // Show summary
//! output.success_summary("2 packages installed");
//! ```

pub mod actor;
pub mod buffer;
pub mod list;
pub mod output;
pub mod progress;
pub mod table;
pub mod theme;

// Re-export main types for convenience
pub mod engine;
pub use engine::RelativeFrame;
pub use output::Output;
pub use theme::Theme;
