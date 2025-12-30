//! Crossterm-based terminal output system
//!
//! Clean, table-based output matching APL mockup design.
//! Uses crossterm for cursor control and colored output.

use crossterm::{
    ExecutableCommand, QueueableCommand, cursor,
    style::{Color, SetForegroundColor, Stylize},
    terminal::{Clear, ClearType},
};
use std::io::{Stdout, Write, stdout};
use std::sync::{Arc, Mutex};

// ============================================================================
// Design System Constants
// ============================================================================

/// Status icons - all single-width for column alignment
pub const STATUS_PENDING: &str = "○";
pub const STATUS_ACTIVE: &str = "●";
pub const STATUS_OK: &str = "✓";
pub const STATUS_ERR: &str = "✗";
pub const STATUS_WARN: &str = "⚠";

/// Column positions for tabular layout
const COL_STATUS: u16 = 0;
const COL_NAME: u16 = 3;
const COL_VERSION: u16 = 18;
const COL_SIZE: u16 = 30;
const COL_PROGRESS: u16 = 42;
const TABLE_WIDTH: usize = 70;

// ============================================================================
// Helper Functions
// ============================================================================

/// Format bytes for display matching mockup (KB/MB)
pub fn format_size(bytes: u64) -> String {
    let kb = bytes as f64 / 1024.0;
    if kb >= 1024.0 {
        format!("{:.1} MB", kb / 1024.0)
    } else if kb >= 1.0 {
        format!("{:.1} KB", kb)
    } else {
        format!("{} B", bytes)
    }
}

/// Write text at a specific position with color
fn write_at(stdout: &mut Stdout, row: u16, col: u16, text: &str, color: Color) {
    let _ = stdout.queue(cursor::MoveTo(col, row));
    let _ = stdout.queue(SetForegroundColor(color));
    print!("{}", text);
    let _ = stdout.queue(SetForegroundColor(Color::Reset));
    let _ = stdout.flush();
}

/// Clear a single line
fn clear_line(stdout: &mut Stdout, row: u16) {
    let _ = stdout.queue(cursor::MoveTo(0, row));
    let _ = stdout.queue(Clear(ClearType::CurrentLine));
    let _ = stdout.flush();
}

// ============================================================================
// Package State
// ============================================================================

#[derive(Clone, Copy, PartialEq)]
pub enum TableMode {
    Standard, // Name, Version, Size, Status
    Update,   // Name, OldVersion -> NewVersion, Status
}

#[derive(Clone, Copy)]
pub enum StandaloneStatus {
    Ok,
    Warn,
    Err,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PackageState {
    Pending,
    Downloading { current: u64, total: u64 },
    Installing,
    Done { detail: String },
    Warn { detail: String },
    Failed { reason: String },
}

#[derive(Clone)]
struct StandaloneRow {
    message: String,
    active: bool,
}

#[derive(Clone)]
struct PackageInfo {
    name: String,
    version: String,
    new_version: Option<String>, // For Update mode
    size: u64,
    state: PackageState,
    row: u16,
}

// ============================================================================
// TableOutput - Main output system
// ============================================================================

pub struct TableOutput {
    packages: Vec<PackageInfo>,
    standalone: Option<StandaloneRow>,
    mode: TableMode,
    base_row: u16,
    frame: usize,
    stdout: Stdout,
}

impl TableOutput {
    pub fn new() -> Self {
        Self {
            packages: Vec::new(),
            standalone: None,
            mode: TableMode::Standard,
            base_row: 0,
            frame: 0,
            stdout: stdout(),
        }
    }

    /// Prepare the terminal for a pipelined command (install, remove, update)
    /// This pre-prints the rows to ensure the terminal scrolls if needed
    /// and anchors the base_row for stable relative positioning.
    pub fn prepare_pipeline(&mut self, items: &[(String, Option<String>)]) {
        self.mode = TableMode::Standard;
        println!();
        let (name_header, ver_header) = ("PACKAGE", "VERSION");

        // Header
        // Header
        println!(
            "   {} {} {} {}",
            format!("{:<14}", name_header).dark_grey(),
            format!("{:<11}", ver_header).dark_grey(),
            format!("{:<11}", "SIZE").dark_grey(),
            "STATUS".dark_grey()
        );
        println!("{}", "─".repeat(TABLE_WIDTH).dark_grey());

        // Reserve space for all items
        for _ in 0..items.len() {
            println!();
        }

        // Calculate base_row based on final cursor position
        let (_, end_row) = cursor::position().unwrap_or((0, 0));
        self.base_row = end_row.saturating_sub(items.len() as u16);

        // Populate package info
        for (i, (name, version)) in items.iter().enumerate() {
            let ver = version.as_deref().unwrap_or("-");

            self.packages.push(PackageInfo {
                name: name.clone(),
                version: ver.to_string(),
                new_version: None,
                size: 0,
                state: PackageState::Pending,
                row: self.base_row + i as u16,
            });

            // Initial render
            self.render_package(self.packages.len() - 1);
        }
    }

    /// Prepare pipeline for UPDATE command (different column layout)
    pub fn prepare_update_pipeline(&mut self, items: &[(String, String, String)]) {
        self.mode = TableMode::Update;
        println!();

        // Header matches apl_mockup: PACKAGE, CURRENT, ->, NEW, STATUS
        // Positions:
        // Pkg: 3
        // Old: 18
        // Arrow: 30
        // New: 33
        // Status: 42

        println!(
            "   {} {} {} {} {}",
            format!("{:<14}", "PACKAGE").dark_grey(),
            format!("{:<11}", "CURRENT").dark_grey(),
            format!("{:<2}", "→").dark_grey(),
            format!("{:<11}", "NEW").dark_grey(),
            "STATUS".dark_grey()
        );
        println!("{}", "─".repeat(TABLE_WIDTH).dark_grey());

        let starting_row = cursor::position().map(|(_, r)| r).unwrap_or(0);
        self.base_row = starting_row;

        for (name, old, new) in items {
            println!(
                " {} {:<14} {:<11} {:<2} {:<11} {}",
                STATUS_PENDING.dark_grey(),
                name.as_str().cyan(),
                old.as_str().dark_grey(),
                "→".dark_grey(),
                new.as_str().green(),
                "pending".dark_grey()
            );

            let current_row = self.packages.len() as u16;
            self.packages.push(PackageInfo {
                name: name.clone(),
                version: old.clone(),
                new_version: Some(new.clone()),
                size: 0,
                state: PackageState::Pending,
                row: starting_row + current_row,
            });
        }
    }

    /// Print table header for install/update commands
    pub fn print_header_install(&mut self) {
        self.mode = TableMode::Standard;
        println!();
        println!(
            "   {} {} {} {}",
            format!("{:<14}", "PACKAGE").dark_grey(),
            format!("{:<11}", "VERSION").dark_grey(),
            format!("{:<11}", "SIZE").dark_grey(),
            "STATUS".dark_grey()
        );
        println!("{}", "─".repeat(TABLE_WIDTH).dark_grey());

        self.base_row = cursor::position().map(|(_, r)| r).unwrap_or(0);
    }

    /// Print table header for list command
    pub fn print_header_list(&mut self) {
        println!();
        println!(
            "   {:<14} {:<11} {:<11} {}",
            "PACKAGE".dark_grey(),
            "VERSION".dark_grey(),
            "SIZE".dark_grey(),
            "INSTALLED".dark_grey()
        );
        println!("{}", "─".repeat(TABLE_WIDTH).dark_grey());
        self.base_row = cursor::position().map(|(_, r)| r).unwrap_or(0);
    }

    /// Add a package to track (prints initial pending row) or update existing
    pub fn add_package(&mut self, name: &str, version: &str, size: u64) -> usize {
        // Check if package already exists (e.g. from prepare_pipeline)
        if let Some(idx) = self.packages.iter().position(|p| p.name == name) {
            let pkg = &mut self.packages[idx];
            pkg.version = version.to_string();
            pkg.size = size;
            // Re-render in place
            self.render_package(idx);
            return idx;
        }

        // New package: print newline and capture position
        println!();
        let (_, row) = cursor::position().unwrap_or((0, 0));
        let pkg_row = row.saturating_sub(1);

        let pkg = PackageInfo {
            name: name.to_string(),
            version: version.to_string(),
            new_version: None,
            size,
            state: PackageState::Pending,
            row: pkg_row,
        };
        self.packages.push(pkg);
        let idx = self.packages.len() - 1;
        self.render_package(idx);
        idx
    }

    /// Set package to downloading state
    pub fn set_downloading(&mut self, name: &str, total: u64) {
        if let Some(pkg) = self.packages.iter_mut().find(|p| p.name == name) {
            pkg.state = PackageState::Downloading { current: 0, total };
            pkg.size = total;
        }
        self.render_package_by_name(name);
    }

    /// Update download progress
    pub fn update_progress(&mut self, name: &str, current: u64) {
        self.frame += 1;
        if let Some(pkg) = self.packages.iter_mut().find(|p| p.name == name) {
            if let PackageState::Downloading { total, .. } = pkg.state {
                pkg.state = PackageState::Downloading { current, total };
            }
        }
        self.render_package_by_name(name);
    }

    /// Set package to installing state
    pub fn set_installing(&mut self, name: &str) {
        self.frame += 1;
        if let Some(pkg) = self.packages.iter_mut().find(|p| p.name == name) {
            pkg.state = PackageState::Installing;
        }
        self.render_package_by_name(name);
    }

    /// Set package to done state. Returns true if package was in table.
    pub fn set_done(&mut self, name: &str, detail: &str) -> bool {
        if let Some(pkg) = self.packages.iter_mut().find(|p| p.name == name) {
            pkg.state = PackageState::Done {
                detail: detail.to_string(),
            };
            self.render_package_by_name(name);
            true
        } else {
            false
        }
    }

    /// Set package to failed state. Returns true if package was in table.
    pub fn set_failed(&mut self, name: &str, reason: &str) -> bool {
        if let Some(pkg) = self.packages.iter_mut().find(|p| p.name == name) {
            pkg.state = PackageState::Failed {
                reason: reason.to_string(),
            };
            self.render_package_by_name(name);
            true
        } else {
            false
        }
    }

    /// Force a re-render of all active items to update animations
    pub fn tick(&mut self) {
        self.frame += 1;

        // Render packages
        for i in 0..self.packages.len() {
            let active = matches!(
                self.packages[i].state,
                PackageState::Downloading { .. } | PackageState::Installing
            );
            if active {
                self.render_package(i);
            }
        }

        // Render standalone
        if let Some(standalone) = &self.standalone {
            if standalone.active {
                self.render_standalone();
            }
        }
    }

    /// Prepare a standalone row (non-tabular)
    pub fn prepare_standalone(&mut self, initial_msg: &str) {
        self.standalone = Some(StandaloneRow {
            message: initial_msg.to_string(),
            active: true,
        });
        self.render_standalone();
    }

    /// Update standalone message
    pub fn update_standalone(&mut self, msg: &str) {
        if let Some(s) = &mut self.standalone {
            s.message = msg.to_string();
            s.active = true;
            self.render_standalone();
        }
    }

    /// Finish standalone row
    pub fn finish_standalone(&mut self, msg: &str, status: StandaloneStatus) {
        if let Some(s) = &mut self.standalone {
            s.message = msg.to_string();
            s.active = false;
            self.render_standalone_final(status);
        }
    }

    fn render_standalone(&mut self) {
        if let Some(s) = &self.standalone {
            let _ = self.stdout.queue(cursor::MoveToColumn(0));
            let _ = self.stdout.queue(Clear(ClearType::CurrentLine));

            let dot = if self.frame % 4 < 2 {
                STATUS_ACTIVE
            } else {
                STATUS_PENDING
            };

            let _ = self.stdout.queue(SetForegroundColor(Color::Red));
            print!("{}", dot);
            let _ = self.stdout.queue(SetForegroundColor(Color::Reset));
            print!("  ");
            let _ = self.stdout.queue(SetForegroundColor(Color::Cyan));
            print!("{}", s.message);
            let _ = self.stdout.queue(SetForegroundColor(Color::Reset));
            let _ = self.stdout.flush();
        }
    }

    fn render_standalone_final(&mut self, status: StandaloneStatus) {
        if let Some(s) = &self.standalone {
            let _ = self.stdout.queue(cursor::MoveToColumn(0));
            let _ = self.stdout.queue(Clear(ClearType::CurrentLine));

            match status {
                StandaloneStatus::Ok => {
                    let _ = self.stdout.queue(SetForegroundColor(Color::Green));
                    print!("{}  {}", STATUS_OK, s.message);
                }
                StandaloneStatus::Warn => {
                    let _ = self.stdout.queue(SetForegroundColor(Color::Yellow));
                    print!("{}  {}", STATUS_WARN, s.message);
                }
                StandaloneStatus::Err => {
                    let _ = self.stdout.queue(SetForegroundColor(Color::Red));
                    print!("{}  {}", STATUS_ERR, s.message);
                }
            }
            let _ = self.stdout.queue(SetForegroundColor(Color::Reset));
            let _ = self.stdout.flush();
            // Finalize line
            println!();
        }
    }

    /// Generic footer drawer
    fn draw_footer(&mut self, message: &str, icon: &str, icon_color: Color, text_color: Color) {
        // Move past all package rows
        let footer_row = self.base_row + self.packages.len() as u16;
        let _ = self.stdout.execute(cursor::MoveTo(0, footer_row));
        println!();
        println!("{}", "─".repeat(TABLE_WIDTH).dark_grey());

        let mut stdout = stdout();
        let row = cursor::position().map(|(_, r)| r).unwrap_or(0);

        write_at(&mut stdout, row, 0, icon, icon_color);
        write_at(&mut stdout, row, 2, message, text_color);
        println!();
        println!();
    }

    /// Print success footer
    pub fn print_success(&mut self, message: &str) {
        self.draw_footer(message, STATUS_OK, Color::Green, Color::Green);
    }

    /// Print warning footer
    pub fn print_warn(&mut self, message: &str) {
        self.draw_footer(message, STATUS_WARN, Color::Yellow, Color::Yellow);
    }

    /// Print error footer
    pub fn print_error(&mut self, message: &str) {
        self.draw_footer(message, STATUS_ERR, Color::Red, Color::Red);
    }

    /// Legacy support for print_footer (maps to success/warn)
    pub fn print_footer(&mut self, message: &str, success: bool, color: Option<Color>) {
        if success {
            self.draw_footer(
                message,
                STATUS_OK,
                Color::Green,
                color.unwrap_or(Color::Green),
            );
        } else {
            self.draw_footer(
                message,
                STATUS_WARN,
                Color::Yellow,
                color.unwrap_or(Color::Yellow),
            );
        }
    }

    fn render_package_by_name(&mut self, name: &str) {
        if let Some(idx) = self.packages.iter().position(|p| p.name == name) {
            self.render_package(idx);
        }
    }

    fn render_package(&mut self, idx: usize) {
        let pkg = &self.packages[idx];
        let row = pkg.row;

        clear_line(&mut self.stdout, row);

        // Status indicator
        match &pkg.state {
            PackageState::Pending => {
                write_at(
                    &mut self.stdout,
                    row,
                    COL_STATUS,
                    STATUS_PENDING,
                    Color::DarkGrey,
                );
            }
            PackageState::Downloading { .. } | PackageState::Installing => {
                // Blinking effect
                let dot = if self.frame % 4 < 2 {
                    STATUS_ACTIVE
                } else {
                    STATUS_PENDING
                };
                write_at(&mut self.stdout, row, COL_STATUS, dot, Color::Red);
            }
            PackageState::Done { .. } => {
                write_at(&mut self.stdout, row, COL_STATUS, STATUS_OK, Color::Green);
            }
            PackageState::Warn { .. } => {
                write_at(
                    &mut self.stdout,
                    row,
                    COL_STATUS,
                    STATUS_WARN,
                    Color::Yellow,
                );
            }
            PackageState::Failed { .. } => {
                write_at(&mut self.stdout, row, COL_STATUS, STATUS_ERR, Color::Red);
            }
        }

        // Name and Version (common to both modes)
        write_at(
            &mut self.stdout,
            row,
            COL_NAME,
            &format!("{:<14}", pkg.name),
            if self.mode == TableMode::Update {
                Color::White
            } else {
                Color::Cyan
            },
        );
        write_at(
            &mut self.stdout,
            row,
            COL_VERSION,
            &format!("{:<11}", pkg.version),
            if self.mode == TableMode::Update {
                Color::DarkGrey
            } else {
                Color::White
            }, // Old version is dark grey in update mode
        );

        match self.mode {
            TableMode::Standard => {
                // Standard: Size column
                write_at(
                    &mut self.stdout,
                    row,
                    COL_SIZE,
                    &format!("{:<11}", format_size(pkg.size)),
                    Color::DarkGrey,
                );
            }
            TableMode::Update => {
                // Update: Arrow and New Version
                write_at(
                    &mut self.stdout,
                    row,
                    30, // Arrow position
                    "→",
                    Color::DarkGrey,
                );
                if let Some(new_ver) = &pkg.new_version {
                    write_at(
                        &mut self.stdout,
                        row,
                        33, // New version position
                        new_ver,
                        Color::Green,
                    );
                }
            }
        }

        // Status text (Common logic, different colors maybe?)
        match &pkg.state {
            PackageState::Pending => {
                write_at(
                    &mut self.stdout,
                    row,
                    COL_PROGRESS,
                    "pending",
                    Color::DarkGrey,
                );
            }
            PackageState::Downloading { current, total } => {
                let pct = if *total > 0 {
                    (*current * 100 / *total).min(100)
                } else {
                    0
                };
                let dl = format_size(*current);

                let action = if self.mode == TableMode::Update {
                    "updating"
                } else {
                    "fetching"
                };

                write_at(
                    &mut self.stdout,
                    row,
                    COL_PROGRESS,
                    &format!("{} {:>3}% {}", action, pct, dl),
                    Color::Cyan,
                );
            }
            PackageState::Installing => {
                let msg = if self.mode == TableMode::Update {
                    "installing"
                } else {
                    "installing"
                };
                write_at(&mut self.stdout, row, COL_PROGRESS, msg, Color::Yellow);
            }
            PackageState::Done { detail } => {
                write_at(&mut self.stdout, row, COL_PROGRESS, detail, Color::Green);
            }
            PackageState::Warn { detail } => {
                write_at(&mut self.stdout, row, COL_PROGRESS, detail, Color::Yellow);
            }
            PackageState::Failed { reason } => {
                write_at(
                    &mut self.stdout,
                    row,
                    COL_PROGRESS,
                    &format!("FAILED: {}", reason),
                    Color::Red,
                );
            }
        }
    }
}

impl Default for TableOutput {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// CliOutput - Thread-safe wrapper for backward compatibility
// ============================================================================

#[derive(Clone)]
pub struct CliOutput {
    inner: Arc<Mutex<TableOutput>>,
}

impl CliOutput {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(TableOutput::new())),
        }
    }

    /// Legacy: Create with PackageProgress (ignored, for compatibility)
    pub fn with_progress(_progress: PackageProgress) -> Self {
        Self::new()
    }

    /// Print section header (simple, no table)
    pub fn section(&self, title: &str) {
        println!();
        println!("{} {}", title, "─".repeat(40));
    }

    /// Print table header for install operations
    pub fn section_table(&self) {
        self.inner.lock().unwrap().print_header_install();
    }

    /// Print table header for list operations
    pub fn section_table_list(&self) {
        self.inner.lock().unwrap().print_header_list();
    }

    /// Success message (footer)
    pub fn success(&self, msg: &str) {
        self.inner.lock().unwrap().print_success(msg);
    }

    /// Info message
    pub fn info(&self, msg: &str) {
        println!("  {} {}", STATUS_PENDING.dark_grey(), msg);
    }

    /// Warning message (footer)
    pub fn warning(&self, msg: &str) {
        self.inner.lock().unwrap().print_warn(msg);
    }

    /// Error message (footer)
    pub fn error(&self, msg: &str) {
        self.inner.lock().unwrap().print_error(msg);
    }

    /// Hint message
    pub fn hint(&self, msg: &str) {
        println!("  💡 {}", msg);
    }

    /// Summary footer with timing
    pub fn summary(&self, count: usize, action: &str, duration_secs: f64) {
        let msg = format!(
            "{} package{} {} in {:.1}s",
            count,
            if count == 1 { "" } else { "s" },
            action,
            duration_secs
        );
        self.success(&msg);
    }

    /// Summary footer without timing (for instantaneous actions)
    pub fn summary_plain(&self, count: usize, action: &str) {
        let msg = format!(
            "{} package{} {}",
            count,
            if count == 1 { "" } else { "s" },
            action
        );
        self.success(&msg);
    }

    /// Print error summary footer (alias for error)
    pub fn error_summary(&self, message: &str) {
        self.error(message);
    }

    /// Add package to table (for install flow)
    pub fn add_package(&self, name: &str, version: &str) {
        self.inner.lock().unwrap().add_package(name, version, 0);
    }

    /// Print footer for list command
    pub fn print_list_footer(&self, message: &str, success: bool, color: Option<Color>) {
        self.inner
            .lock()
            .unwrap()
            .print_footer(message, success, color);
    }

    /// Prepare standalone inline row
    pub fn prepare_standalone(&self, msg: &str) {
        self.inner.lock().unwrap().prepare_standalone(msg);
    }

    /// Update standalone inline row
    pub fn update_standalone(&self, msg: &str) {
        self.inner.lock().unwrap().update_standalone(msg);
    }

    /// Finish standalone inline row
    pub fn finish_standalone(&self, msg: &str, status: StandaloneStatus) {
        self.inner.lock().unwrap().finish_standalone(msg, status);
    }

    /// Set downloading with size
    pub fn set_downloading(&self, name: &str, _version: &str, total: u64) {
        self.inner.lock().unwrap().set_downloading(name, total);
    }

    /// Update download progress
    pub fn update_download(&self, name: &str, current: u64) {
        self.inner.lock().unwrap().update_progress(name, current);
    }

    /// Set installing state
    pub fn set_installing(&self, name: &str, _version: &str) {
        self.inner.lock().unwrap().set_installing(name);
    }

    /// Mark as done
    pub fn done(&self, name: &str, version: &str, detail: &str) {
        let found = self.inner.lock().unwrap().set_done(name, detail);
        if !found {
            // Standalone output for packages not in table
            use crossterm::style::Stylize;
            println!(
                "{} {:<14} {:<11} {}",
                STATUS_OK.green(),
                name.cyan(),
                version.dark_grey(),
                detail.green()
            );
        }
    }

    /// Alias for done
    pub fn finish_ok(&self, name: &str, version: &str, detail: &str) {
        self.done(name, version, detail);
    }

    /// Mark as failed
    pub fn finish_err(&self, name: &str, version: &str, reason: &str) {
        let found = self.inner.lock().unwrap().set_failed(name, reason);
        if !found {
            // Standalone output for packages not in table
            use crossterm::style::Stylize;
            println!(
                "{} {:<14} {:<11} {}",
                STATUS_ERR.red(),
                name.cyan(),
                version.dark_grey(),
                reason.red()
            );
        }
    }

    /// Alias for finish_err
    pub fn fail(&self, name: &str, version: &str, reason: &str) {
        self.finish_err(name, version, reason);
    }

    /// Verbose message (for switch operations)
    pub fn verbose(&self, msg: &str) {
        println!("  {}", msg);
    }

    /// Pre-allocate packages (used by install pipeline)
    pub fn prepare_pipeline(&self, packages: &[(String, Option<String>)]) {
        let mut inner = self.inner.lock().unwrap();
        inner.prepare_pipeline(packages);
    }

    /// Pre-allocate packages for UPDATE pipeline
    pub fn prepare_update_pipeline(&self, packages: &[(String, String, String)]) {
        let mut inner = self.inner.lock().unwrap();
        inner.prepare_update_pipeline(packages);
    }

    /// Start a background task to refresh the UI (for animations)
    pub fn start_tick(&self) -> tokio::task::JoinHandle<()> {
        let inner = self.inner.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(250));
            loop {
                interval.tick().await;
                let mut table = match inner.lock() {
                    Ok(t) => t,
                    Err(_) => break,
                };
                table.tick();
            }
        })
    }
}

impl Default for CliOutput {
    fn default() -> Self {
        Self::new()
    }
}

// Placeholder for backward compatibility
#[derive(Clone)]
pub struct PackageProgress;

impl PackageProgress {
    pub fn new() -> Self {
        Self
    }
}

impl Default for PackageProgress {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// List Output - Simple table for listing packages
// ============================================================================

// Column widths derived from mockup positions:
// COL_NAME=3, COL_VERSION=18 (15 chars), COL_SIZE=30 (12 chars), COL_PROGRESS=42
const NAME_WIDTH: usize = 15; // 18 - 3 = 15
const VERSION_WIDTH: usize = 12; // 30 - 18 = 12
const SIZE_WIDTH: usize = 12; // 42 - 30 = 12

/// Print a single package row in list format with an optional status symbol
pub fn print_list_row(name: &str, version: &str, size: u64, status: &str, symbol: &str) {
    use crossterm::style::Stylize;

    let size_str = if size > 0 {
        format_size(size)
    } else {
        "-".to_string()
    };

    // Pad strings BEFORE styling to get correct alignment
    let name_padded = format!("{:<width$}", name, width = NAME_WIDTH);
    let version_padded = format!("{:<width$}", version, width = VERSION_WIDTH);
    let size_padded = format!("{:<width$}", size_str, width = SIZE_WIDTH);

    // Apply color to symbol based on content
    let symbol_styled = match symbol {
        "✓" | "✔" => symbol.green(),
        "✗" | "!" | "✘" => symbol.red(),
        "↑" => symbol.cyan(),
        _ => symbol.dark_grey(),
    };

    println!(
        "{:<2} {}{}{}{}",
        symbol_styled,
        name_padded.cyan(),
        version_padded.white(),
        size_padded.dark_grey(),
        status.dark_grey()
    );
}

/// Print list table header
pub fn print_list_header() {
    use crossterm::style::Stylize;

    println!();

    // Pad strings BEFORE styling
    let pkg_header = format!("{:<width$}", "PACKAGE", width = NAME_WIDTH);
    let ver_header = format!("{:<width$}", "VERSION", width = VERSION_WIDTH);
    let size_header = format!("{:<width$}", "SIZE", width = SIZE_WIDTH);

    println!(
        "   {}{}{}{}",
        pkg_header.dark_grey(),
        ver_header.dark_grey(),
        size_header.dark_grey(),
        "INSTALLED".dark_grey()
    );
    println!("{}", "─".repeat(TABLE_WIDTH).dark_grey());
}

/// Print list table footer
pub fn print_list_footer(count: usize, total_size: u64) {
    use crossterm::style::Stylize;

    println!();
    println!("{}", "─".repeat(TABLE_WIDTH).dark_grey());

    let size_str = if total_size > 0 {
        format!(" ({})", format_size(total_size))
    } else {
        String::new()
    };
    let msg = format!(
        "{} package{} installed{}",
        count,
        if count == 1 { "" } else { "s" },
        size_str
    );
    println!("{} {}", STATUS_OK.green(), msg.green());
    println!();
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_size_bytes() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1), "1 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1023), "1023 B");
    }

    #[test]
    fn test_format_size_kilobytes() {
        assert_eq!(format_size(1024), "1.0 KB");
        assert_eq!(format_size(1536), "1.5 KB");
        assert_eq!(format_size(10240), "10.0 KB");
    }

    #[test]
    fn test_format_size_megabytes() {
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
        assert_eq!(format_size(1024 * 1024 * 5), "5.0 MB");
    }

    #[test]
    fn test_format_size_gigabytes() {
        assert_eq!(format_size(1024 * 1024 * 1024), "1.0 GB");
    }
}
