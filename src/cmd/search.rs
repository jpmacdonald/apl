//! Search command

use anyhow::{Context, Result, bail};
use apl::apl_home;
use apl::index::PackageIndex;

/// Search packages in the local index (U.S. Graphics style output)
pub fn search(query: &str) -> Result<()> {
    let start = std::time::Instant::now();
    let index_path = apl_home().join("index");
    if !index_path.exists() {
        bail!("No index found. Run 'apl update' first.");
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
    // U.S. Graphics: Subtle section title
    println!("{}", "Searching repositories".dark_grey());
    println!();

    // Rows (no headers)
    for entry in &results {
        let name = format!("{:<width$}", entry.name, width = theme.layout.name_width);
        let version = format!(
            "{:<width$}",
            entry.latest().map(|v| v.version.as_str()).unwrap_or("?"),
            width = theme.layout.version_width
        );
        let description = entry.description.as_str();

        println!(
            "  {} {} {}",
            name.with(theme.colors.package_name),
            version.with(theme.colors.version),
            description.with(theme.colors.secondary)
        );
    }

    // Footer: count and timing
    let elapsed = start.elapsed();
    println!();
    println!(
        "{}",
        format!(
            "{} packages found ({:.2}s)",
            results.len(),
            elapsed.as_secs_f64()
        )
        .dark_grey()
    );

    Ok(())
}
