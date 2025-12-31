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
    let size_str = super::theme::format_size(current);
    format!("fetching {pct:>3}% {size_str}")
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
        assert_eq!(format_download_progress(512, 1024), "fetching  50% 512 B");
        assert_eq!(format_download_progress(1024, 1024), "fetching 100% 1.0 KB");
        assert_eq!(format_download_progress(0, 1024), "fetching   0% 0 B");
    }
}
