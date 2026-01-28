//! List and search output formatting
//!
//! Provides column-aligned rendering for `apl list` and `apl search`.

use super::buffer::OutputBuffer;
use super::theme::Theme;
use crossterm::style::Stylize;

/// Print column headers for `apl list`
pub fn print_list_header(buffer: &mut OutputBuffer) {
    let theme = Theme::default();

    buffer.write_line("", theme.colors.header);

    let header = format!(
        "  {:<nw$} {:<vw$} {:>8}   {}",
        "name",
        "version",
        "size",
        "installed",
        nw = theme.layout.name_width,
        vw = theme.layout.version_width,
    );
    buffer.write_line(&header.dark_grey().to_string(), theme.colors.header);
}

/// Print a single row for `apl list`
pub fn print_list_row(
    buffer: &mut OutputBuffer,
    name: &str,
    version: &str,
    size: u64,
    date: &str,
) {
    let theme = Theme::default();

    let name_part = format!("{:<width$}", name, width = theme.layout.name_width);
    let version_part = format!("{:<width$}", version, width = theme.layout.version_width);
    let size_part = if size > 0 {
        format!("{:>8}", super::theme::format_size(size))
    } else {
        format!("{:>8}", "")
    };

    let line = format!(
        "  {} {} {}   {}",
        name_part.with(theme.colors.package_name),
        version_part.with(theme.colors.version),
        size_part.with(theme.colors.secondary),
        date.with(theme.colors.secondary)
    );
    buffer.write_line(&line, theme.colors.secondary);
}

/// Print footer for `apl list`
pub fn print_list_footer(buffer: &mut OutputBuffer, count: usize, total_size: u64) {
    let theme = Theme::default();

    buffer.write_line("", theme.colors.border);

    let size_str = super::theme::format_size(total_size);
    let msg = format!("  {count} packages, {size_str}");
    buffer.write_line(&msg.dark_grey().to_string(), theme.colors.secondary);
}

/// Print column headers for `apl search`
pub fn print_search_header(buffer: &mut OutputBuffer) {
    let theme = Theme::default();

    let header = format!(
        "  {:<nw$} {:<vw$} {}",
        "name",
        "version",
        "description",
        nw = theme.layout.name_width,
        vw = theme.layout.version_width,
    );
    buffer.write_line(&header.dark_grey().to_string(), theme.colors.header);
}

/// Print a single row for `apl search`
pub fn print_search_row(
    buffer: &mut OutputBuffer,
    name: &str,
    version: &str,
    description: &str,
) {
    let theme = Theme::default();

    let name_part = format!("{:<width$}", name, width = theme.layout.name_width);
    let version_part = format!("{:<width$}", version, width = theme.layout.version_width);

    let line = format!(
        "  {} {} {}",
        name_part.with(theme.colors.package_name),
        version_part.with(theme.colors.version),
        description.with(theme.colors.secondary)
    );
    buffer.write_line(&line, theme.colors.secondary);
}
