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

/// Format progress status with word-based Mission Control style (no bars)
pub fn format_progress_status(current: u64, total: Option<u64>) -> String {
    // Treat None or Some(0) as indeterminate
    let total_valid = total.filter(|&t| t > 0);

    let status_word = if current == 0 {
        "queued"
    } else if let Some(t) = total_valid {
        if current >= t {
            "installed"
        } else {
            "fetching..."
        }
    } else {
        "verifying..."
    };

    let size_str = if current > 0 {
        super::theme::format_size(current)
    } else {
        " ".to_string()
    };

    // Pad everything to exactly 50 characters total
    // Mission Control: "fetching...  1.2 MB"
    let combined = format!("{status_word: <15} {size_str:>10}");
    format!("{combined:<50}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_progress_animation() {
        let mut progress = ProgressIndicator::new(Icons::default());
        let initial_frame = progress.frame();

        // tick() is now a no-op, it shouldn't change the frame
        progress.tick();
        assert_eq!(progress.frame(), initial_frame);
    }

    #[test]
    fn test_progress_status_format() {
        // 50% progress download
        let result = format_progress_status(512, Some(1024));
        assert!(result.contains("fetching..."));
        assert!(result.contains("512 B"));

        // 100% progress
        let result = format_progress_status(1024, Some(1024));
        assert!(result.contains("installed"));
        assert!(result.contains("1.0 KB"));

        // 0% progress
        let result = format_progress_status(0, Some(1024));
        assert!(result.contains("queued"));
    }

    #[test]
    fn test_progress_bar_format_removed() {
        // We verify that the structure is still padded correctly
        let result = format_progress_status(0, None);
        assert_eq!(result.len(), 50);
    }
}
