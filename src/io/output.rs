//! Real-time progress output for apl using indicatif
//!
//! Key patterns (validated by tests/progress_test.rs):
//! 1. Create all bars BEFORE spawning async tasks
//! 2. Clone bar handles to pass into tasks
//! 3. Always call finish() on every bar before scope ends
//! 4. Never use println during operation - use mp.println() or wait

use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const NAME_WIDTH: usize = 14;
const VERSION_WIDTH: usize = 10;

/// Progress tracker for package operations
#[derive(Clone)]
pub struct PackageProgress {
    mp: Arc<MultiProgress>,
    bars: Arc<Mutex<HashMap<String, ProgressBar>>>,
}

impl PackageProgress {
    pub fn new() -> Self {
        Self {
            mp: Arc::new(MultiProgress::new()),
            bars: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Add a placeholder for a package (Pending state)
    pub fn add_package(&self, name: &str, version: &str) -> ProgressBar {
        let style = ProgressStyle::default_spinner()
            .template(&format!(
                "  {{spinner}} {{prefix:<{NAME_WIDTH}}} {{msg:<{VERSION_WIDTH}}}  pending"
            ))
            .unwrap()
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ ");

        let pb = self.mp.add(ProgressBar::new(0));
        pb.set_style(style);
        pb.set_prefix(name.to_string());
        pb.set_message(version.to_string());
        pb.enable_steady_tick(Duration::from_millis(80));

        self.bars
            .lock()
            .unwrap()
            .insert(name.to_string(), pb.clone());
        pb
    }

    /// Transition to "downloading..." state with progress bar
    pub fn set_downloading(&self, name: &str, version: &str, total_bytes: u64) {
        if let Some(pb) = self.bars.lock().unwrap().get(name) {
            let style = ProgressStyle::default_bar()
                .template(&format!(
                    "  ⠸ {{prefix:<{NAME_WIDTH}}} {{msg:<{VERSION_WIDTH}}}  [{{bar:15}}] {{bytes}}/{{total_bytes}}"
                ))
                .unwrap()
                .progress_chars("═╸ ");
            pb.set_style(style);
            pb.set_message(version.to_string());
            pb.set_length(total_bytes);
        }
    }

    /// Transition bar to "installing..." state
    pub fn set_installing(&self, name: &str, version: &str) {
        if let Some(pb) = self.bars.lock().unwrap().get(name) {
            let style = ProgressStyle::default_spinner()
                .template(&format!(
                    "  {{spinner}} {{prefix:<{NAME_WIDTH}}} {{msg:<{VERSION_WIDTH}}}  installing..."
                ))
                .unwrap()
                .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ ");
            pb.set_style(style);
            pb.set_message(version.to_string());
        }
    }

    /// Transition bar to "done" state (finished)
    pub fn set_done(&self, name: &str, version: &str) {
        if let Some(pb) = self.bars.lock().unwrap().get(name) {
            let style = ProgressStyle::default_spinner()
                .template(&format!(
                    "  ✔ {{prefix:<{NAME_WIDTH}}} {{msg:<{VERSION_WIDTH}}}  done"
                ))
                .unwrap();
            pb.set_style(style);
            pb.set_message(version.to_string());
            pb.finish();
        }
    }

    /// Transition bar to failed state
    pub fn set_failed(&self, name: &str, version: &str, reason: &str) {
        if let Some(pb) = self.bars.lock().unwrap().get(name) {
            let style = ProgressStyle::default_spinner()
                .template(&format!(
                    "  ✘ {{prefix:<{NAME_WIDTH}}} {{msg:<{VERSION_WIDTH}}}  {}",
                    reason
                ))
                .unwrap();
            pb.set_style(style);
            pb.set_message(version.to_string());
            pb.finish();
        }
    }

    /// Get a bar handle by name
    pub fn get(&self, name: &str) -> Option<ProgressBar> {
        self.bars.lock().unwrap().get(name).cloned()
    }

    /// Finish all bars
    pub fn finish_all(&self) {
        for pb in self.bars.lock().unwrap().values() {
            if !pb.is_finished() {
                pb.finish();
            }
        }
    }

    pub fn print_summary(&self, count: usize, action: &str, duration_secs: f64) {
        self.finish_all();
        println!();
        println!(
            "  {} {} package{} in {:.1}s",
            if action == "installed" {
                "Installed"
            } else {
                "Removed"
            },
            count,
            if count == 1 { "" } else { "s" },
            duration_secs
        );
    }

    pub fn print_warning(&self, msg: &str) {
        self.mp.println(format!("  ⚠ {}", msg)).ok();
    }

    pub fn print_section(&self, title: &str) {
        self.mp
            .println(format!("{} {}", title, "━".repeat(40)))
            .ok();
    }
}

/// Format bytes as human readable
pub fn format_size(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;

    if bytes >= GIB {
        format!("{:.1} GiB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{} B", bytes)
    }
}

// ============================================================================
// Legacy InstallOutput - Now wraps PackageProgress for consistent UI
// ============================================================================

#[derive(Clone)]
pub struct InstallOutput {
    progress: PackageProgress,
}

impl InstallOutput {
    pub fn new(_verbose: bool) -> Self {
        Self {
            progress: PackageProgress::new(),
        }
    }

    pub fn section(&self, title: &str) {
        self.progress.print_section(title);
    }

    pub fn done(&self, name: &str, version: &str, _detail: &str) {
        if self.progress.get(name).is_none() {
            self.progress.add_package(name, version);
        }
        self.progress.set_done(name, version);
    }

    pub fn warn(&self, msg: &str) {
        self.progress.print_warning(msg);
    }

    pub fn error(&self, msg: &str) {
        eprintln!("  ✘ {}", msg);
    }

    pub fn hint(&self, msg: &str) {
        println!("  💡 {}", msg);
    }

    pub fn summary(&self, count: usize, action: &str, duration: f64) {
        self.progress.print_summary(count, action, duration);
    }

    pub fn download_bar(&self, name: &str, version: &str, total: u64) -> ProgressBar {
        let pb = self.progress.add_package(name, version);
        self.progress.set_downloading(name, version, total);
        pb
    }

    pub fn finish_ok(&self, name: &str, version: &str, size_or_detail: &str) {
        if let Some(pb) = self.progress.get(name) {
            let style = ProgressStyle::default_spinner()
                .template(&format!(
                    "  ✔ {{prefix:<{NAME_WIDTH}}} {{msg:<{VERSION_WIDTH}}}  {}",
                    size_or_detail
                ))
                .unwrap();
            pb.set_style(style);
            pb.set_message(version.to_string());
            pb.finish();
        }
    }

    pub fn finish_err(&self, name: &str, version: &str, detail: &str) {
        self.progress.set_failed(name, version, detail);
    }

    pub fn spinner(&self, name: &str, version: &str, msg: &str) -> ProgressBar {
        let pb = self.progress.get(name).unwrap_or_else(|| {
            let pb = self.progress.add_package(name, version);
            pb
        });

        let style = ProgressStyle::default_spinner()
            .template(&format!(
                "  {{spinner}} {{prefix:<{NAME_WIDTH}}} {{msg:<{VERSION_WIDTH}}}  {}",
                msg
            ))
            .unwrap()
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏ ");
        pb.set_style(style);
        pb
    }

    pub fn fail(&self, name: &str, version: &str, detail: &str) {
        self.progress.set_failed(name, version, detail);
    }

    pub fn verbose(&self, _msg: &str) {
        // Potentially use mp.println if we want verbose output to not flicker
    }

    pub fn prepare_pipeline(&self, packages: &[(String, Option<String>)]) {
        for (name, version) in packages {
            self.progress
                .add_package(name, version.as_deref().unwrap_or(""));
        }
    }
}
