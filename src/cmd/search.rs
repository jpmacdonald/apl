//! Search command

use anyhow::{Context, Result, bail};
use apl::apl_home;
use apl::index::PackageIndex;

/// Search packages in the local index
pub fn search(query: &str) -> Result<()> {
    let index_path = apl_home().join("index.bin");
    if !index_path.exists() {
        bail!("No index found. Run 'dl update' first.");
    }

    let index = PackageIndex::load(&index_path).context("Failed to load index")?;

    let results = index.search(query);

    let theme = apl::ui::Theme::default();
    use crossterm::style::Stylize;

    if results.is_empty() {
        println!();
        println!(
            "  {} No packages found matching '{}'",
            theme.icons.info.blue(),
            query.white()
        );
        println!();
        return Ok(());
    }

    println!();
    // Header
    println!(
        "   {} {} {}",
        format!("{:<width$}", "PACKAGE", width = theme.layout.name_width).dark_grey(),
        format!("{:<width$}", "VERSION", width = theme.layout.version_width).dark_grey(),
        "DESCRIPTION".dark_grey()
    );
    println!("{}", "─".repeat(theme.layout.table_width).dark_grey());

    // Rows
    for entry in results {
        let name = format!("{:<width$}", entry.name, width = theme.layout.name_width);
        let version = format!(
            "{:<width$}",
            entry.latest().version,
            width = theme.layout.version_width
        );
        let description = &entry.description;

        println!(
            "   {} {} {}",
            name.with(theme.colors.package_name),
            version.with(theme.colors.version),
            description.clone().with(theme.colors.secondary)
        );
    }

    // Footer
    println!("{}", "─".repeat(theme.layout.table_width).dark_grey());
    println!();

    Ok(())
}
