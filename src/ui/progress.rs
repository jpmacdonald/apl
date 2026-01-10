//! Progress Indicators and Spinners
//!
//! This module provides visual feedback for ongoing operations.

use super::theme::Icons;

/// Progress indicator state
#[derive(Debug, Clone)]
pub struct ProgressIndicator {
    frame: usize,
    icons: Icons,
}

impl ProgressIndicator {
    /// Create a new progress indicator
    pub fn new(icons: Icons) -> Self {
        Self { frame: 0, icons }
    }

    /// Get the current animation frame (for spinners)
    pub fn current_icon(&self) -> &'static str {
        // Blink between active and pending every 2 frames
        if self.frame % 4 < 2 {
            self.icons.active
        } else {
            self.icons.pending
        }
    }

    /// Advance to next animation frame
    pub fn tick(&mut self) {
        self.frame = self.frame.wrapping_add(1);
    }

    /// Get current frame number
    pub fn frame(&self) -> usize {
        self.frame
    }
}

impl Default for ProgressIndicator {
    fn default() -> Self {
        Self::new(Icons::default())
    }
}

/// Format download progress (percentage and size)
pub fn format_download_progress(current: u64, total: u64) -> String {
    let pct = if total > 0 {
        (current * 100 / total).min(100)
    } else {
        0
    };
    let bar = format_progress_bar(current, total, 24);
    let size_str = super::theme::format_size(total);
    format!("{bar}  {pct:>3}%  {size_str}")
}

/// Format a 24-character progress bar using ▓ (filled) and ░ (empty).
/// This is the U.S. Graphics Company style: clean, minimal Unicode.
pub fn format_progress_bar(current: u64, total: u64, width: usize) -> String {
    let filled = if total > 0 {
        ((current as f64 / total as f64) * width as f64).round() as usize
    } else {
        0
    };
    let empty = width.saturating_sub(filled);
    format!("{}{}", "▓".repeat(filled), "░".repeat(empty))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_progress_animation() {
        let mut progress = ProgressIndicator::default();
        assert_eq!(progress.frame(), 0);

        progress.tick();
        assert_eq!(progress.frame(), 1);

        progress.tick();
        assert_eq!(progress.frame(), 2);
    }

    #[test]
    fn test_download_progress_format() {
        // 50% progress on a 1024 byte download
        let result = format_download_progress(512, 1024);
        assert!(result.contains("▓"));
        assert!(result.contains("░"));
        assert!(result.contains("50%"));
        assert!(result.contains("1.0 KB")); // total size

        // 100% progress
        let result = format_download_progress(1024, 1024);
        assert!(result.contains("100%"));
        assert!(!result.contains("░")); // Should be all filled

        // 0% progress
        let result = format_download_progress(0, 1024);
        assert!(result.contains("0%"));
        assert!(!result.contains("▓")); // Should be all empty
    }

    #[test]
    fn test_progress_bar_format() {
        // 50% should be half filled
        let bar = super::format_progress_bar(50, 100, 10);
        assert_eq!(bar.chars().filter(|c| *c == '▓').count(), 5);
        assert_eq!(bar.chars().filter(|c| *c == '░').count(), 5);

        // 100% should be all filled
        let bar = super::format_progress_bar(100, 100, 10);
        assert_eq!(bar.chars().filter(|c| *c == '▓').count(), 10);
        assert_eq!(bar.chars().filter(|c| *c == '░').count(), 0);

        // 0% should be all empty
        let bar = super::format_progress_bar(0, 100, 10);
        assert_eq!(bar.chars().filter(|c| *c == '▓').count(), 0);
        assert_eq!(bar.chars().filter(|c| *c == '░').count(), 10);
    }
}
