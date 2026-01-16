//! Terminal Engine - Relative positioning for real-time UIs
//!
//! This module solves the "where is my cursor" problem by using relative
//! coordinates. We print N lines of space and then move UP relative to
//! the current cursor position to draw.

use crossterm::{
    QueueableCommand,
    cursor::{MoveDown, MoveToColumn, MoveUp},
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

        // 1. Move to the row
        self.stdout.queue(MoveToColumn(0))?;
        if row_idx > 0 {
            self.stdout.queue(MoveDown(row_idx))?;
        }

        // 2. Clear the line
        self.stdout.queue(Clear(ClearType::CurrentLine))?;

        // 3. Render the content
        f(&mut self.stdout)?;

        // 4. Move back to the top of the frame
        self.stdout.queue(MoveToColumn(0))?;
        if row_idx > 0 {
            self.stdout.queue(MoveUp(row_idx))?;
        }

        // NOTE: We do NOT flush here anymore to allow batching.
        // Caller must call .flush() explicitly.
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
        // Move to the very bottom to leave space for the next command prompt
        self.stdout.queue(MoveDown(self.total_rows))?;
        self.stdout.queue(MoveToColumn(0))?;
        self.stdout.flush()?;
        Ok(())
    }
}
