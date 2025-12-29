//! Polished console output for apl
//!
//! Uses indicatif for animated spinners and progress bars with fixed-width columns.

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use console::style;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

// Fixed column widths for alignment
const NAME_WIDTH: usize = 12;
const VERSION_WIDTH: usize = 10;

/// Styled output for install/remove operations
pub struct InstallOutput {
    verbose: bool,
    mp: MultiProgress,
    bars: Arc<Mutex<HashMap<String, ProgressBar>>>,
}

impl InstallOutput {
    pub fn new(verbose: bool) -> Self {
        Self {
            verbose,
            mp: MultiProgress::new(),
            bars: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Print section header
    pub fn section(&self, title: &str) {
        self.mp.suspend(|| {
            let bar = "━".repeat(45 - title.len());
            println!("{} {}", style(title).bold(), style(bar).dim());
        });
    }

    /// Create a download progress bar with spinner, percent, and bar
    /// Template: "  ⠋ hyperfine      1.19.0      45%  [━━━━━░░░░░]"
    pub fn download_bar(&self, name: &str, version: &str, total_size: u64) -> ProgressBar {
        let mut bars = self.bars.lock().unwrap();
        
        // Style for downloading: spinner + percent + bar
        let pb_style = ProgressStyle::default_bar()
            .template(&format!(
                "  {{spinner:.dim}} {{prefix:<{NAME_WIDTH}}}  {{msg:>{VERSION_WIDTH}}}  {{percent:>3}}%  [{{bar:10.cyan/dim}}]"
            ))
            .unwrap()
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏✔")
            .progress_chars("━░");
        
        let pb = self.mp.add(ProgressBar::new(total_size));
        pb.set_style(pb_style);
        pb.set_prefix(format!("{}", style(name).cyan()));
        pb.set_message(format!("{}", style(version).dim()));
        pb.enable_steady_tick(Duration::from_millis(80));
        
        bars.insert(name.to_string(), pb.clone());
        pb
    }

    /// Create a spinner for operations like "Moving to Applications..."
    pub fn spinner(&self, name: &str, version: &str, msg: &str) -> ProgressBar {
        let mut bars = self.bars.lock().unwrap();
        
        let pb_style = ProgressStyle::default_spinner()
            .template(&format!(
                "  {{spinner:.dim}} {{prefix:<{NAME_WIDTH}}}  {{msg:>{VERSION_WIDTH}}}  {{wide_msg}}"
            ))
            .unwrap()
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏✔");
        
        let pb = self.mp.add(ProgressBar::new_spinner());
        pb.set_style(pb_style);
        pb.set_prefix(format!("{}", style(name).cyan()));
        pb.set_message(format!("{}", style(version).dim()));
        pb.enable_steady_tick(Duration::from_millis(80));
        
        // Store detail in position field (hacky but works)
        pb.println(format!("    {}", style(msg).dim()));
        
        bars.insert(name.to_string(), pb.clone());
        pb
    }

    /// Finish a bar with success (green check)
    pub fn finish_ok(&self, name: &str, version: &str, detail: &str) {
        let bars = self.bars.lock().unwrap();
        if let Some(pb) = bars.get(name) {
            // Clear the progress bar and print the final message explicitly
            // This ensures the output is locked in place
            pb.finish_and_clear();
        }
        drop(bars);
        
        // Print the final state using suspend to ensure it's visible
        self.mp.suspend(|| {
            println!("  {} {:<NAME_WIDTH$}  {:>VERSION_WIDTH$}  {}", 
                style("✔").green(),
                style(name).cyan(), 
                style(version).dim(), 
                style(detail).dim()
            );
        });
    }

    /// Finish a bar with failure (red X)
    pub fn finish_err(&self, name: &str, version: &str, detail: &str) {
        let bars = self.bars.lock().unwrap();
        if let Some(pb) = bars.get(name) {
            pb.finish_and_clear();
        }
        drop(bars);
        
        self.mp.suspend(|| {
            println!("  {} {:<NAME_WIDTH$}  {:>VERSION_WIDTH$}  {}", 
                style("✗").red(),
                style(name).cyan(), 
                style(version).dim(), 
                style(detail).red()
            );
        });
    }

    /// Simple done line (no bar needed)
    pub fn done(&self, name: &str, version: &str, detail: &str) {
        self.mp.suspend(|| {
            println!("  {} {:<NAME_WIDTH$}  {:>VERSION_WIDTH$}  {}", 
                style("✔").green(),
                style(name).cyan(), 
                style(version).dim(), 
                style(detail).dim()
            );
        });
    }

    /// Simple fail line (no bar needed)
    pub fn fail(&self, name: &str, version: &str, detail: &str) {
        self.mp.suspend(|| {
            println!("  {} {:<NAME_WIDTH$}  {:>VERSION_WIDTH$}  {}", 
                style("✗").red(),
                style(name).cyan(), 
                style(version).dim(), 
                style(detail).red()
            );
        });
    }

    /// Clear all bars (for section transitions) - prints blank line to seal section
    pub fn clear_section(&self) {
        let mut bars = self.bars.lock().unwrap();
        for (_, pb) in bars.drain() {
            if !pb.is_finished() {
                pb.finish();
            }
        }
        // Print blank line to separate sections
        drop(bars);
        self.mp.suspend(|| println!());
    }

    /// Verbose output
    pub fn verbose(&self, msg: &str) {
        if self.verbose {
            self.mp.suspend(|| {
                println!("    {}", style(msg).dim());
            });
        }
    }

    /// Print summary
    pub fn summary(&self, count: usize, action: &str, duration_secs: f64) {
        self.mp.suspend(|| {
            println!();
            println!("  {} {} package{} {} in {:.1}s",
                style("✨").green(),
                count,
                if count == 1 { "" } else { "s" },
                action,
                duration_secs,
            );
        });
    }

    /// Warning message
    pub fn warn(&self, msg: &str) {
        self.mp.suspend(|| {
            println!("  {} {}", style("⚠").yellow(), msg);
        });
    }

    /// Error message  
    pub fn error(&self, msg: &str) {
        self.mp.suspend(|| {
            println!("  {} {}", style("✗").red(), msg);
        });
    }

    /// Hint message
    pub fn hint(&self, msg: &str) {
        self.mp.suspend(|| {
            println!("  {} {}", style("💡").dim(), style(msg).dim());
        });
    }

    /// Get the MultiProgress for external use
    pub fn mp(&self) -> &MultiProgress {
        &self.mp
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
