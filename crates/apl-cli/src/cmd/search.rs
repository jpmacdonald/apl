//! Search command

use anyhow::{Context, Result, bail};
use apl_core::paths::apl_home;
use apl_schema::index::PackageIndex;

/// Search packages in the local index (U.S. Graphics style output)
pub fn search(query: &str) -> Result<()> {
    use crossterm::style::Stylize;

    let start = std::time::Instant::now();
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

    for entry in &results {
        let name = entry.name.to_string();
        let version = entry.latest().map_or("?", |v| v.version.as_str());
        let description = &entry.description;

        crate::ui::list::print_list_row(&mut buffer, &name, version, 0, description, " ");
    }

    buffer.flush();

    // Footer: Mission Control standardized summary
    let elapsed = start.elapsed();
    println!();
    println!(
        "SEARCH COMPLETE {}, elapsed {:.2}s",
        results.len(),
        elapsed.as_secs_f64()
    );

    Ok(())
}
