//! Table rendering using frame-based relative positioning
//!
//! Renders package rows with animated progress indicators.
//! Uses `RelativeFrame` for in-place updates without terminal corruption.

use super::buffer::OutputBuffer;
use super::engine::RelativeFrame;
use super::progress::ProgressIndicator;
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
    Downloading { current: u64, total: Option<u64> },
    /// Currently installing/extracting
    Installing { current: u64, total: Option<u64> },
    /// Extracting with progress
    Extracting { current: u64, total: Option<u64> },
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
#[derive(Clone, Debug)]
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
    /// Update: Name | Old Version -> New Version | Status
    Update,
}

/// Table renderer for package operations
#[derive(Debug)]
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
        self.packages.clear();
        self.mode = TableMode::Standard;

        for (name, version, depth) in items {
            let ver = version.as_ref().map_or("-", apl_schema::Version::as_str);
            self.packages.push(PackageRow {
                name: name.clone(),
                version: ver.to_string(),
                size: 0,
                state: PackageState::Pending,
                depth: *depth,
            });
        }

        println!();

        let mut frame = RelativeFrame::new(self.packages.len() as u16);
        let _ = frame.start();
        self.frame = Some(frame);

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
                if s > 0 {
                    pkg.size = s;
                }
            }
        }
    }

    /// Get the last known total size for a package
    pub fn get_package_size(&self, name: &PackageName) -> Option<u64> {
        self.packages
            .iter()
            .find(|p| p.name == *name)
            .map(|p| p.size)
            .filter(|&s| s > 0)
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
                    | PackageState::Installing { .. }
                    | PackageState::Extracting { .. }
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
                | PackageState::Installing { .. }
                | PackageState::Extracting { .. }
                | PackageState::Removing => self.progress.current_icon(),
                PackageState::Done { .. } => theme.icons.success,
                PackageState::Warn { .. } => theme.icons.warning,
                PackageState::Failed { .. } => theme.icons.error,
            };

            let _ = frame.write_row(idx as u16, |stdout| {
                // Determine colors from theme
                let (icon_color, name_color, status_color) = match &pkg.state {
                    PackageState::Pending => (
                        theme.colors.secondary,
                        theme.colors.package_name,
                        theme.colors.secondary,
                    ),
                    PackageState::Downloading { .. }
                    | PackageState::Installing { .. }
                    | PackageState::Extracting { .. }
                    | PackageState::Removing => (
                        theme.colors.error,
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

                // Size column: right-aligned in 8 chars
                let size_text = if pkg.size > 0 {
                    format!("{:>8}", super::theme::format_size(pkg.size))
                } else {
                    format!("{:>8}", "")
                };

                // Status text: bare words, padded to fixed width to prevent flashing
                let status_text = match &pkg.state {
                    PackageState::Pending => format!("{:<30}", "queued"),
                    PackageState::Downloading { current, total } => {
                        super::progress::format_progress_status(*current, *total)
                    }
                    PackageState::Extracting { .. } => {
                        format!("{:<30}", "unpacking")
                    }
                    PackageState::Installing { .. } => {
                        format!("{:<30}", "linking")
                    }
                    PackageState::Removing => format!("{:<30}", "removing"),
                    PackageState::Done { detail } | PackageState::Warn { detail } => {
                        format!("{detail:<30}")
                    }
                    PackageState::Failed { reason } => {
                        let msg = format!("FAILED: {reason}");
                        format!("{msg:<30}")
                    }
                };

                // For Done state, the checkmark icon is sufficient -- no status word needed
                let show_status = !matches!(pkg.state, PackageState::Done { .. });

                let prefix = if pkg.depth > 0 {
                    format!("{:indent$}└─ ", "", indent = pkg.depth * 2)
                } else {
                    String::new()
                };

                let visible_len = 2
                    + prefix.chars().count()
                    + icon_str.chars().count()
                    + 1
                    + pkg.name.chars().count();
                let padding_len = theme.layout.phase_padding.saturating_sub(visible_len);
                let padding = " ".repeat(padding_len);

                let version_part = format!(
                    "{: <width$}",
                    pkg.version,
                    width = theme.layout.version_width
                );

                let line = if show_status {
                    format!(
                        "  {}{}{}{}{} {} {} {}",
                        prefix,
                        icon_str.with(icon_color),
                        " ",
                        pkg.name.with(name_color),
                        padding,
                        version_part.with(theme.colors.version),
                        size_text.with(theme.colors.secondary),
                        status_text.with(status_color)
                    )
                } else {
                    format!(
                        "  {}{}{}{}{} {} {}",
                        prefix,
                        icon_str.with(icon_color),
                        " ",
                        pkg.name.with(name_color),
                        padding,
                        version_part.with(theme.colors.version),
                        size_text.with(theme.colors.secondary),
                    )
                };

                write!(stdout, "{line}")?;
                Ok(())
            });
        }
    }

    /// Print footer message with explicit severity (no separator)
    pub fn print_footer(&mut self, buffer: &mut OutputBuffer, message: &str, severity: Severity) {
        // Render one last time to show final state (e.g. Done instead of Pending)
        self.render_all(buffer);

        // Only show cursor if we had an active frame (i.e. an animated table)
        if let Some(mut frame) = self.frame.take() {
            let _ = frame.finish();
            buffer.show_cursor();
        }

        println!();

        match severity {
            Severity::Success => {
                println!("{} {}", self.theme.icons.success.green(), message.green());
            }
            Severity::Warning => {
                println!("{} {}", self.theme.icons.warning.yellow(), message.yellow());
            }
            Severity::Error => println!("{} {}", self.theme.icons.error.red(), message.red()),
        }
    }

    /// Print footer message without icon (plain)
    pub fn print_plain(&mut self, buffer: &mut OutputBuffer, message: &str) {
        // Render one last time to show final state
        self.render_all(buffer);

        // Only show cursor if we had an active frame
        if let Some(mut frame) = self.frame.take() {
            let _ = frame.finish();
            buffer.show_cursor();
        }

        println!();
        println!("{}", message.green());
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
