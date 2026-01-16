//! Search command

use apl_core::paths::apl_home;
use anyhow::{Context, Result, bail};
use apl_schema::index::PackageIndex;

/// Search packages in the local index (U.S. Graphics style output)
pub fn search(query: &str) -> Result<()> {
    let start = std::time::Instant::now();
    let index_path = apl_home().join("index");
    if !index_path.exists() {
        bail!("No index found. Run 'apl update' first.");
    }

    let index = PackageIndex::load(&index_path).context("Failed to load index")?;

    let results = index.search(query);

    let theme = crate::ui::Theme::default();
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

    let mut table = comfy_table::Table::new();
    table.load_preset(comfy_table::presets::NOTHING);
    table.set_content_arrangement(comfy_table::ContentArrangement::Dynamic);
    table.set_width(theme.layout.table_width as u16);

    for entry in &results {
        let name = entry.name.to_string().with(theme.colors.package_name);
        let version = entry
            .latest()
            .map(|v| v.version.as_str())
            .unwrap_or("?")
            .to_string()
            .with(theme.colors.version);
        let description = entry.description.as_str().with(theme.colors.secondary);

        table.add_row(vec![
            comfy_table::Cell::new(format!("  {name}")),
            comfy_table::Cell::new(version),
            comfy_table::Cell::new(description),
        ]);
    }

    println!("{table}");

    // Footer: Mission Control standardized summary
    let elapsed = start.elapsed();
    println!();
    println!(
        "SEARCH COMPLETE {}, elapsed {:.2}s",
        results.len(),
        elapsed.as_secs_f64()
    );

    // JSON RESULT for CI automation
    let result_json = serde_json::json!({
        "operation": "search",
        "query": query,
        "count": results.len(),
        "elapsed": elapsed.as_secs_f64()
    });
    println!("\nRESULT {}", serde_json::to_string(&result_json)?);

    Ok(())
}
