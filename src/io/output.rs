//! Styled console output for apl
//!
//! Clean Grid UI with separated phases and aligned columns.

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use console::style;

/// Package install state for display
#[derive(Clone, Copy, PartialEq)]
pub enum PackageState {
    Queued,
    Downloading,
    Downloaded,
    Installing,
    Installed,
    Failed,
}

impl PackageState {
    pub fn symbol(&self) -> &'static str {
        match self {
            Self::Queued => "○",
            Self::Downloading => "◐",
            Self::Downloaded => "●",
            Self::Installing => "◐",
            Self::Installed => "✔",
            Self::Failed => "✗",
        }
    }
}

/// Styled output for install operations
pub struct InstallOutput {
    verbose: bool,
}

impl InstallOutput {
    pub fn new(verbose: bool) -> Self {
        Self {
            verbose,
        }
    }

    /// Print section header
    pub fn section(&self, title: &str) {
        println!();
        let bar = "━".repeat(45 - title.len());
        println!("{} {}", style(title).bold(), style(bar).dim());
    }

    /// Print package line with state
    pub fn package_line(&self, state: PackageState, name: &str, version: &str, detail: &str) {
        let symbol = match state {
            PackageState::Installed => style(state.symbol()).green(),
            PackageState::Failed => style(state.symbol()).red(),
            _ => style(state.symbol()).dim(),
        };
        
        // Fixed-width columns: symbol(3) name(16) version(12) detail(rest)
        println!("  {} {:<16} {:>10}  {}", 
            symbol,
            style(name).cyan(),
            style(version).dim(),
            style(detail).dim(),
        );
    }

    /// Print verbose-only message (hidden unless --verbose)
    pub fn verbose(&self, msg: &str) {
        if self.verbose {
            println!("    {}", style(msg).dim());
        }
    }

    /// Print success summary
    pub fn summary(&self, count: usize, duration_secs: f64) {
        println!();
        println!("  {} {} package{} installed in {:.1}s",
            style("✨").green(),
            count,
            if count == 1 { "" } else { "s" },
            duration_secs,
        );
    }

    /// Print warning
    pub fn warn(&self, msg: &str) {
        println!("  {} {}", style("⚠").yellow(), msg);
    }

    /// Print error
    pub fn error(&self, msg: &str) {
        println!("  {} {}", style("✗").red(), msg);
    }

    /// Print hint
    pub fn hint(&self, msg: &str) {
        println!("  {} {}", style("💡").dim(), style(msg).dim());
    }

    /// Create aligned progress bar for downloads
    pub fn create_progress(&self, mp: &MultiProgress, name: &str, version: &str, total: u64) -> ProgressBar {
        let pb = mp.add(ProgressBar::new(total));
        
        // Clean style: ┈ name  version  [━━━━╸      ]  50%
        let template = format!(
            "  {} {{prefix:<16}} {{msg:>10}}  [{{bar:20.dim}}] {{percent:>3}}%",
            style("┈").dim()
        );
        
        let style = ProgressStyle::default_bar()
            .template(&template)
            .unwrap()
            .progress_chars("━╸ ");
        
        pb.set_style(style);
        pb.set_prefix(name.to_string());
        pb.set_message(version.to_string());
        pb.enable_steady_tick(std::time::Duration::from_millis(100));
        pb
    }
    
    /// Finish progress bar with success
    pub fn finish_progress_ok(&self, pb: &ProgressBar, size_str: &str) {
        // Update to show completed state
        let template = format!(
            "  {} {{prefix:<16}} {{msg:>10}}",
            style("✔").green()
        );
        
        let style = ProgressStyle::default_bar()
            .template(&template)
            .unwrap();
        
        pb.set_style(style);
        pb.set_message(size_str.to_string());
        pb.finish();
    }
}

/// Format bytes as human-readable
pub fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    }
}
