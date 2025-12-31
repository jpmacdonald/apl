//! UI Theme - Design system constants
//!
//! This module defines all visual elements used in APL's UI:
//! - Colors
//! - Icons  
//! - Column positions
//! - Table dimensions
//!
//! Centralizing these makes it easy to:
//! - Maintain visual consistency
//! - Update the design system
//! - Add themes/color schemes in the future

use crossterm::style::Color;

/// Default theme for APL UI
#[derive(Debug, Clone)]
pub struct Theme {
    /// Colors for different UI elements
    pub colors: ColorScheme,
    /// Status icons
    pub icons: Icons,
    /// Table layout constants
    pub layout: Layout,
}

impl Theme {
    /// Create the default APL theme
    pub fn default() -> Self {
        Self {
            colors: ColorScheme::default(),
            icons: Icons::default(),
            layout: Layout::default(),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::default()
    }
}

/// Color scheme for UI elements
#[derive(Debug, Clone)]
pub struct ColorScheme {
    /// Package names (primary content)
    pub package_name: Color,
    /// Version numbers
    pub version: Color,
    /// Sizes and secondary info
    pub secondary: Color,
    /// Headers and labels
    pub header: Color,
    /// Success states
    pub success: Color,
    /// Warning states
    pub warning: Color,
    /// Error states
    pub error: Color,
    /// Active/in-progress items
    pub active: Color,
    /// Borders and separators
    pub border: Color,
}

impl Default for ColorScheme {
    fn default() -> Self {
        Self {
            package_name: Color::Cyan,
            version: Color::White,
            secondary: Color::DarkGrey,
            header: Color::DarkGrey,
            success: Color::Green,
            warning: Color::Yellow,
            error: Color::Red,
            active: Color::Red,
            border: Color::DarkGrey,
        }
    }
}

/// Status icons for different states
#[derive(Debug, Clone)]
pub struct Icons {
    /// Pending/queued state (○)
    pub pending: &'static str,
    /// Active/in-progress state (●)
    pub active: &'static str,
    /// Success/completed state (✓)
    pub success: &'static str,
    /// Error/failed state (✗)
    pub error: &'static str,
    /// Warning state (⚠)
    pub warning: &'static str,
    /// Info/Tip state (ℹ)
    pub info: &'static str,
}

impl Default for Icons {
    fn default() -> Self {
        Self {
            pending: "○",
            active: "●",
            success: "✓",
            error: "✗",
            warning: "⚠",
            info: "ℹ",
        }
    }
}

/// Table layout constants
#[derive(Debug, Clone)]
pub struct Layout {
    /// Column position for status icon
    pub col_status: u16,
    /// Column position for package name
    pub col_name: u16,
    /// Column position for version
    pub col_version: u16,
    /// Column position for size
    pub col_size: u16,
    /// Column position for progress/status text
    pub col_progress: u16,
    /// Total table width for borders
    pub table_width: usize,
    /// Width allocated for package name column
    pub name_width: usize,
    /// Width allocated for version column
    pub version_width: usize,
    /// Width allocated for size column
    pub size_width: usize,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            col_status: 0,
            col_name: 3,
            col_version: 20,
            col_size: 33,
            col_progress: 44,
            table_width: 70,
            name_width: 16,
            version_width: 12,
            size_width: 10,
        }
    }
}

/// Format bytes for human-readable display
pub fn format_size(bytes: u64) -> String {
    let kb = bytes as f64 / 1024.0;
    let mb = kb / 1024.0;
    if mb >= 1024.0 {
        format!("{:.1} GB", mb / 1024.0)
    } else if kb >= 1024.0 {
        format!("{mb:.1} MB")
    } else if kb >= 1.0 {
        format!("{kb:.1} KB")
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_size_bytes() {
        assert_eq!(format_size(0), "0 B");
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

    #[test]
    fn test_theme_defaults() {
        let theme = Theme::default();
        assert_eq!(theme.icons.success, "✓");
        assert_eq!(theme.icons.error, "✗");
        assert_eq!(theme.layout.table_width, 70);
    }
}
