//! Progress Indicators and Spinners
//!
//! This module provides visual feedback for ongoing operations.

use super::theme::Icons;
use std::time::Instant;

/// Progress indicator state
#[derive(Debug, Clone)]
pub struct ProgressIndicator {
    start_time: Instant,
    icons: Icons,
}

impl ProgressIndicator {
    /// Create a new progress indicator
    pub fn new(icons: Icons) -> Self {
        Self {
            start_time: Instant::now(),
            icons,
        }
    }

    /// Get the current animation frame (for spinners)
    /// Uses wall-clock time so animation is independent of render frequency
    pub fn current_icon(&self) -> &'static str {
        // 200ms per frame = 5 FPS blink rate
        let elapsed_ms = self.start_time.elapsed().as_millis() as usize;
        let frame = elapsed_ms / 200;

        // Blink between active and pending every 2 frames
        if frame % 2 == 0 {
            self.icons.active
        } else {
            self.icons.pending
        }
    }

    /// Advance to next animation frame (no-op for time-based animation)
    pub fn tick(&mut self) {
        // No-op: animation is now time-based
    }

    /// Get current frame number (for testing)
    pub fn frame(&self) -> usize {
        let elapsed_ms = self.start_time.elapsed().as_millis() as usize;
        elapsed_ms / 200
    }
}

impl Default for ProgressIndicator {
    fn default() -> Self {
        Self::new(Icons::default())
    }
}

/// Format progress status with consistent layout and padding
pub fn format_progress_status(current: u64, total: Option<u64>) -> String {
    let bar_width = 24;

    // Treat None or Some(0) as indeterminate
    let total_valid = total.filter(|&t| t > 0);

    let bar = if let Some(t) = total_valid {
        format_progress_bar(current, t, bar_width)
    } else {
        format_progress_bar(0, 100, bar_width)
    };

    let pct_str = if let Some(t) = total_valid {
        let pct = (current * 100 / t).min(100);
        format!(" {:>3}% ", pct)
    } else {
        "  ... ".to_string()
    };

    let size_str = if current > 0 {
        super::theme::format_size(current)
    } else {
        "0.0  B".to_string()
    };

    // Pad everything to exactly 50 characters total to ensure zero-flicker overwrites
    let combined = format!("{} {} {:<10}", bar, pct_str, size_str);
    format!("{:<50}", combined)
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
    fn test_progress_status_format() {
        // 50% progress on a 1024 byte download
        let result = format_progress_status(512, Some(1024));
        assert!(result.contains("▓"));
        assert!(result.contains("░"));
        assert!(result.contains("50%"));
        assert!(result.contains("1.0 KB"));

        // 100% progress
        let result = format_progress_status(1024, Some(1024));
        assert!(result.contains("100%"));
        assert!(!result.contains("░"));

        // 0% progress
        let result = format_progress_status(0, Some(1024));
        assert!(result.contains("0%"));
        assert!(!result.contains("▓"));
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
