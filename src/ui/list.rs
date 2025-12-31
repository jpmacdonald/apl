//! List output formatting
//!
//! Uses OutputBuffer for atomic and consistent rendering.

use super::buffer::OutputBuffer;
use super::theme::{Theme, format_size};
use crossterm::style::Stylize;

/// Print list table header
pub fn print_list_header(buffer: &mut OutputBuffer) {
    let theme = Theme::default();
    buffer.write_line(&format!("{}", ""), theme.colors.header); // Empty line spacer

    // Pad the strings BEFORE applying color
    let pkg_header = format!("{: <width$}", "PACKAGE", width = theme.layout.name_width);
    let ver_header = format!("{: <width$}", "VERSION", width = theme.layout.version_width);
    let size_header = format!("{: <width$}", "SIZE", width = theme.layout.size_width);

    // Apply spacing between columns without colors affecting width
    let line = format!(
        "   {} {} {} {}",
        pkg_header.with(theme.colors.header),
        ver_header.with(theme.colors.header),
        size_header.with(theme.colors.header),
        "INSTALLED".with(theme.colors.header)
    );
    buffer.write_line(&line, theme.colors.header);

    let separator = "─".repeat(theme.layout.table_width);
    buffer.write_line(&separator, theme.colors.border);
}

/// Print a single package row
pub fn print_list_row(
    buffer: &mut OutputBuffer,
    name: &str,
    version: &str,
    size: u64,
    status: &str,
    symbol: &str,
) {
    let theme = Theme::default();
    let size_str = if size > 0 {
        format_size(size)
    } else {
        "-".to_string()
    };

    // Format with proper padding BEFORE applying colors
    let name_part = format!("{: <width$}", name, width = theme.layout.name_width);
    let version_part = format!("{: <width$}", version, width = theme.layout.version_width);
    let size_part = format!("{: <width$}", size_str, width = theme.layout.size_width);

    let symbol_styled = match symbol {
        "✓" | "✔" => symbol.with(theme.colors.success),
        "✗" | "!" | "✘" | "error" => symbol.with(theme.colors.error),
        "↑" => symbol.with(theme.colors.active),
        _ => " ".with(theme.colors.secondary),
    };

    // Print with proper spacing
    let line = format!(
        "{}  {} {} {} {}",
        symbol_styled,
        name_part.with(theme.colors.package_name),
        version_part.with(theme.colors.version),
        size_part.with(theme.colors.secondary),
        status.with(theme.colors.secondary)
    );
    buffer.write_line(&line, theme.colors.secondary);
}

/// Print list table footer
pub fn print_list_footer(buffer: &mut OutputBuffer, count: usize, total_size: u64) {
    let theme = Theme::default();

    let separator = "─".repeat(theme.layout.table_width);
    buffer.write_line("", theme.colors.border);
    buffer.write_line(&separator, theme.colors.border);

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
    let footer = format!(
        "{} {}",
        theme.icons.success.with(theme.colors.success),
        msg.with(theme.colors.success)
    );
    buffer.write_line(&footer, theme.colors.success);
    buffer.write_line("", theme.colors.success);
}
