//! Output Buffer - Atomic terminal rendering
//!
//! This module provides buffering for terminal output to ensure
//! that frames are rendered atomically without interleaving.
//!
//! The key insight is that multiple threads calling `println!()` directly
//! causes corruption. Instead, we buffer all output and flush it atomically.

use crossterm::{
    QueueableCommand, cursor, execute,
    style::{Color, SetForegroundColor},
    terminal::{Clear, ClearType},
};
use std::io::{Stdout, Write};

/// A buffer that accumulates terminal commands before flushing
pub struct OutputBuffer {
    stdout: Stdout,
}

impl OutputBuffer {
    /// Create a new output buffer
    pub fn new(stdout: Stdout) -> Self {
        Self { stdout }
    }

    /// Clear a specific line
    pub fn clear_line(&mut self, row: u16) {
        let _ = self.stdout.queue(cursor::MoveTo(0, row));
        let _ = self.stdout.queue(Clear(ClearType::CurrentLine));
    }

    /// Write text at a specific position with color
    pub fn write_at(&mut self, row: u16, col: u16, text: &str, color: Color) {
        let _ = self.stdout.queue(cursor::MoveTo(col, row));
        let _ = self.stdout.queue(SetForegroundColor(color));
        let _ = write!(self.stdout, "{}", text);
        let _ = self.stdout.queue(SetForegroundColor(Color::Reset));
    }

    /// Write text on a new line with color
    pub fn write_line(&mut self, text: &str, color: Color) {
        let _ = self.stdout.queue(SetForegroundColor(color));
        let _ = writeln!(self.stdout, "{}", text);
        let _ = self.stdout.queue(SetForegroundColor(Color::Reset));
    }

    /// Write text without newline
    pub fn write(&mut self, text: &str) {
        let _ = write!(self.stdout, "{}", text);
    }

    /// Move cursor to position
    pub fn move_to(&mut self, col: u16, row: u16) {
        let _ = self.stdout.queue(cursor::MoveTo(col, row));
    }

    /// Hide cursor
    pub fn hide_cursor(&mut self) {
        let _ = execute!(self.stdout, cursor::Hide);
    }

    /// Show cursor
    pub fn show_cursor(&mut self) {
        let _ = execute!(self.stdout, cursor::Show);
    }

    /// Flush all queued commands to terminal (atomic render)
    pub fn flush(&mut self) {
        let _ = self.stdout.flush();
    }
}

impl Default for OutputBuffer {
    fn default() -> Self {
        Self::new(std::io::stdout())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_buffer_creation() {
        let _buffer = OutputBuffer::default();
        // Just verify it can be created
        assert!(true);
    }
}
