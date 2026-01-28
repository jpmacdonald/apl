//! Search command

use anyhow::{Context, Result, bail};
use apl_core::paths::apl_home;
use apl_schema::index::PackageIndex;

/// Search packages in the local index
pub fn search(query: &str) -> Result<()> {
    use crossterm::style::Stylize;

    let index_path = apl_home().join("index");
    if !index_path.exists() {
        bail!("No index found. Run 'apl update' first.");
    }

    let index = PackageIndex::load(&index_path).context("Failed to load index")?;

    let results = index.search(query);

    let theme = crate::ui::Theme::default();

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

    let mut buffer = crate::ui::buffer::OutputBuffer::default();
    println!();

    crate::ui::list::print_search_header(&mut buffer);

    for entry in &results {
        let name = entry.name.to_string();
        let version = entry.latest().map_or("?", |v| v.version.as_str());
        let description = &entry.description;

        crate::ui::list::print_search_row(&mut buffer, &name, version, description);
    }

    buffer.flush();

    println!();
    let count = results.len();
    let plural = if count == 1 { "" } else { "s" };
    println!("  {count} result{plural}");

    Ok(())
}
