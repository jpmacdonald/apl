//! Table Rendering - Milspec real-time table
//!
//! This module handles rendering the package table using a frame-based
//! relative positioning system. This ensures smooth updates without
//! terminal corruption.

use super::buffer::OutputBuffer;
use super::engine::RelativeFrame;
use super::progress::{ProgressIndicator, format_download_progress};
use super::theme::Theme;
use apl_schema::types::{PackageName, Version};
use crossterm::style::Stylize;
use std::io::Write;

/// Package state during operations
#[derive(Clone, Debug, PartialEq)]
pub enum PackageState {
    /// Queued/waiting
    Pending,
    /// Currently downloading
    Downloading { current: u64, total: u64 },
    /// Currently installing/extracting
    Installing,
    /// Currently removing
    Removing,
    /// Successfully completed
    Done { detail: String },
    /// Completed with warning
    Warn { detail: String },
    /// Failed with error
    Failed { reason: String },
}

/// A single package row in the table
#[derive(Clone)]
struct PackageRow {
    name: PackageName,
    version: String,
    state: PackageState,
    size: u64,
    depth: usize,
}

/// Table mode determines column layout
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum TableMode {
    /// Standard: Name | Version | Size | Status
    Standard,
    /// Update: Name | Old Version → New Version | Status
    Update,
}

/// Table renderer for package operations
pub struct TableRenderer {
    packages: Vec<PackageRow>,
    mode: TableMode,
    theme: Theme,
    progress: ProgressIndicator,
    frame: Option<RelativeFrame>,
}

impl TableRenderer {
    /// Create a new table renderer
    pub fn new(theme: Theme) -> Self {
        Self {
            packages: Vec::new(),
            mode: TableMode::Standard,
            theme,
            progress: ProgressIndicator::default(),
            frame: None,
        }
    }

    /// Prepare table for a pipeline of packages
    pub fn prepare_pipeline(
        &mut self,
        buffer: &mut OutputBuffer,
        items: &[(PackageName, Option<Version>, usize)],
    ) {
        buffer.hide_cursor();
        // 1. Clear old state
        self.packages.clear();
        self.mode = TableMode::Standard;

        // 2. Initialize package data
        for (name, version, depth) in items {
            let ver = version.as_ref().map(|v| v.as_str()).unwrap_or("-");
            self.packages.push(PackageRow {
                name: name.clone(),
                version: ver.to_string(),
                size: 0,
                state: PackageState::Pending,
                depth: *depth,
            });
        }

        // 3. U.S. Graphics Style: No headers, no separators. Clean whitespace.
        println!();

        // 4. Initialize the Frame for the rows
        let mut frame = RelativeFrame::new(self.packages.len() as u16);
        let _ = frame.start();
        self.frame = Some(frame);

        // 5. Initial render
        self.render_all(buffer);
    }

    /// Update a package's state by name
    pub fn update_package(
        &mut self,
        name: &PackageName,
        version: Option<&Version>,
        state: PackageState,
        size: Option<u64>,
    ) {
        if let Some(pkg) = self.packages.iter_mut().find(|p| p.name == *name) {
            pkg.state = state;
            if let Some(v) = version {
                pkg.version = v.to_string();
            }
            if let Some(s) = size {
                pkg.size = s;
            }
        }
    }

    /// Render all active (animating) packages
    pub fn render_active(&mut self, _buffer: &mut OutputBuffer) {
        self.progress.tick();

        // We only need to re-render rows that are in an active state (blinking)
        let mut rows_to_render = Vec::new();
        for (i, pkg) in self.packages.iter().enumerate() {
            if matches!(
                pkg.state,
                PackageState::Downloading { .. }
                    | PackageState::Installing
                    | PackageState::Removing
            ) {
                rows_to_render.push(i);
            }
        }

        if !rows_to_render.is_empty() {
            for idx in rows_to_render {
                self.render_single_row(idx);
            }
            if let Some(frame) = self.frame.as_mut() {
                let _ = frame.flush();
            }
        }
    }

    /// Render all packages (full table refresh)
    pub fn render_all(&mut self, _buffer: &mut OutputBuffer) {
        for i in 0..self.packages.len() {
            self.render_single_row(i);
        }
        if let Some(frame) = self.frame.as_mut() {
            let _ = frame.flush();
        }
    }

    fn render_single_row(&mut self, idx: usize) {
        if let Some(frame) = self.frame.as_mut() {
            let pkg = self.packages[idx].clone();
            let theme = self.theme.clone();

            let icon_str = match &pkg.state {
                PackageState::Pending => theme.icons.pending,
                PackageState::Downloading { .. }
                | PackageState::Installing
                | PackageState::Removing => self.progress.current_icon(),
                PackageState::Done { .. } => theme.icons.success,
                PackageState::Warn { .. } => theme.icons.warning,
                PackageState::Failed { .. } => theme.icons.error,
            };

            let _ = frame.write_row(idx as u16, |stdout| {
                // Determine colors from theme
                let (_icon_color, name_color, status_color) = match &pkg.state {
                    PackageState::Pending => (
                        theme.colors.secondary,
                        theme.colors.package_name,
                        theme.colors.secondary,
                    ),
                    PackageState::Downloading { .. } => (
                        theme.colors.active,
                        theme.colors.package_name,
                        theme.colors.secondary, // Neutral status text
                    ),
                    PackageState::Installing => (
                        theme.colors.active, // Red icon
                        theme.colors.package_name,
                        theme.colors.secondary, // Neutral status text
                    ),
                    PackageState::Removing => (
                        theme.colors.active, // Red icon
                        theme.colors.package_name,
                        theme.colors.secondary, // Neutral status text
                    ),
                    PackageState::Done { .. } => (
                        theme.colors.success,
                        theme.colors.package_name,
                        theme.colors.success,
                    ),
                    PackageState::Warn { .. } => (
                        theme.colors.warning,
                        theme.colors.package_name,
                        theme.colors.warning,
                    ),
                    PackageState::Failed { .. } => (
                        theme.colors.error,
                        theme.colors.package_name,
                        theme.colors.error,
                    ),
                };

                // Format status
                let status_text = match &pkg.state {
                    PackageState::Pending => "pending".to_string(),
                    PackageState::Downloading { current, total } => {
                        format_download_progress(*current, *total)
                    }
                    PackageState::Installing => "installing...".to_string(),
                    PackageState::Removing => "removing...".to_string(),
                    PackageState::Done { detail } => detail.clone(),
                    PackageState::Warn { detail } => detail.clone(),
                    PackageState::Failed { reason } => format!("FAILED: {reason}"),
                };

                // (Size column removed for Mission Control style)

                // Mission Control formatting: 2-space indent for top-level,
                // +2 spaces and └─ for children.
                let prefix = if pkg.depth > 0 {
                    format!("{:indent$}└─ ", "", indent = pkg.depth * 2)
                } else {
                    "".to_string()
                };

                let name_full = format!("  {}{} {}", prefix, icon_str, pkg.name);
                let name_part =
                    format!("{: <width$}", name_full, width = theme.layout.phase_padding);
                let version_part = format!(
                    "{: <width$}",
                    pkg.version,
                    width = theme.layout.version_width
                );

                // Build line
                let line = format!(
                    "{} {} {}",
                    name_part.with(name_color),
                    version_part.with(theme.colors.version),
                    status_text.with(status_color)
                );

                write!(stdout, "{line}")?;
                Ok(())
            });
        }
    }

    /// Print footer message with explicit severity (no separator)
    pub fn print_footer(&mut self, buffer: &mut OutputBuffer, message: &str, severity: Severity) {
        if let Some(mut frame) = self.frame.take() {
            let _ = frame.finish();
        }
        buffer.show_cursor();

        // U.S. Graphics: No separator, just clean whitespace
        println!();

        match severity {
            Severity::Success => {
                println!("{} {}", self.theme.icons.success.green(), message.green())
            }
            Severity::Warning => {
                println!("{} {}", self.theme.icons.warning.yellow(), message.yellow())
            }
            Severity::Error => println!("{} {}", self.theme.icons.error.red(), message.red()),
        }
    }
}

/// Message severity for footer
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Severity {
    Success,
    Warning,
    Error,
}

impl Default for TableRenderer {
    fn default() -> Self {
        Self::new(Theme::default())
    }
}

impl Drop for TableRenderer {
    fn drop(&mut self) {
        if let Some(mut frame) = self.frame.take() {
            let _ = frame.finish();
        }
    }
}
