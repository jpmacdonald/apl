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

    let output = apl::ui::Output::new();

    if results.is_empty() {
        output.info(&format!("No packages found matching '{query}'"));
        return Ok(());
    }

    output.section(&format!("Packages matching '{query}'"));
    for entry in results {
        println!(
            "  {:<14} {:<10} — {}",
            entry.name,
            entry.latest().version,
            entry.description
        );
    }

    Ok(())
}
