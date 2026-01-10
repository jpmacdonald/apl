//! List output formatting
//!
//! Uses OutputBuffer for atomic and consistent rendering.

use super::buffer::OutputBuffer;
use super::theme::Theme;
use crossterm::style::Stylize;

/// Print list table header (U.S. Graphics: just whitespace, no headers)
pub fn print_list_header(buffer: &mut OutputBuffer) {
    let theme = Theme::default();
    buffer.write_line("", theme.colors.header);

    // U.S. Graphics style: Print a subtle section title, no column headers
    let title = "Installed packages";
    buffer.write_line(&format!("{}", title.dark_grey()), theme.colors.header);
    buffer.write_line("", theme.colors.header);
}

/// Print a single package row in U.S. Graphics style
pub fn print_list_row(
    buffer: &mut OutputBuffer,
    name: &str,
    version: &str,
    _size: u64,
    description: &str,
    _symbol: &str,
) {
    let theme = Theme::default();

    // U.S. Graphics style: Name | Version | Description (clean, no icons)
    let name_part = format!("{: <width$}", name, width = theme.layout.name_width);
    let version_part = format!("{: <width$}", version, width = theme.layout.version_width);

    let line = format!(
        "  {} {} {}",
        name_part.with(theme.colors.package_name),
        version_part.with(theme.colors.version),
        description.with(theme.colors.secondary)
    );
    buffer.write_line(&line, theme.colors.secondary);
}

/// Print list table footer (U.S. Graphics: clean summary, no separator)
pub fn print_list_footer(buffer: &mut OutputBuffer, count: usize, _total_size: u64) {
    let theme = Theme::default();

    buffer.write_line("", theme.colors.border);

    let msg = format!("{} packages total", count);
    buffer.write_line(&msg.dark_grey().to_string(), theme.colors.secondary);
    buffer.write_line("", theme.colors.success);
}
