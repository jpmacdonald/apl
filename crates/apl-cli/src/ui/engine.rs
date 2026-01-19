//! Terminal Engine - Relative positioning for real-time UIs
//!
//! This module solves the "where is my cursor" problem by using relative
//! coordinates. We print N lines of space and then move UP relative to
//! the current cursor position to draw.

use crossterm::{
    QueueableCommand,
    cursor::{MoveDown, MoveToColumn, MoveUp, RestorePosition, SavePosition},
    terminal::{Clear, ClearType},
};
use std::io::{Result, Stdout, Write, stdout};

pub struct RelativeFrame {
    stdout: Stdout,
    total_rows: u16,
    started: bool,
}

impl RelativeFrame {
    pub fn new(rows: u16) -> Self {
        Self {
            stdout: stdout(),
            total_rows: rows,
            started: false,
        }
    }

    /// Prepare the terminal space by printing the rows
    pub fn start(&mut self) -> Result<()> {
        // Print N lines of space
        for _ in 0..self.total_rows {
            writeln!(self.stdout)?;
        }
        // Move cursor back up to the top of our frame
        self.stdout.queue(MoveUp(self.total_rows))?;
        self.stdout.queue(MoveToColumn(0))?;
        // Save this anchor position
        self.stdout.queue(SavePosition)?;
        self.stdout.flush()?;
        self.started = true;
        Ok(())
    }

    /// Jump to a specific row (0..total_rows) and perform an operation
    pub fn write_row(
        &mut self,
        row_idx: u16,
        f: impl FnOnce(&mut Stdout) -> Result<()>,
    ) -> Result<()> {
        if !self.started {
            self.start()?;
        }

        if row_idx >= self.total_rows {
            return Ok(());
        }

        // 1. Restore to anchor (top-left of frame)
        self.stdout.queue(RestorePosition)?;

        // 2. Move to the specific row
        if row_idx > 0 {
            self.stdout.queue(MoveDown(row_idx))?;
        }

        // 3. Move to column 0 (don't clear yet - prevents flash)
        self.stdout.queue(MoveToColumn(0))?;

        // 4. Render the content
        f(&mut self.stdout)?;

        // 5. Clear from cursor to end of line (cleans up leftover chars)
        self.stdout.queue(Clear(ClearType::UntilNewLine))?;

        // 6. Restore to anchor again to remain clean
        self.stdout.queue(RestorePosition)?;

        // NOTE: We do NOT flush here anymore to allow batching.
        Ok(())
    }

    /// Flush pending changes to the terminal
    pub fn flush(&mut self) -> Result<()> {
        self.stdout.flush()?;
        Ok(())
    }

    /// Finish the frame and position cursor at the bottom
    pub fn finish(&mut self) -> Result<()> {
        if !self.started {
            return Ok(());
        }
        // Restore to anchor
        self.stdout.queue(RestorePosition)?;
        // Move down past the frame
        self.stdout.queue(MoveDown(self.total_rows))?;
        self.stdout.queue(MoveToColumn(0))?;
        self.stdout.flush()?;
        Ok(())
    }
}
