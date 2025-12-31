//! Tool to update packages automatically
//! Usage: cargo run --bin update_packages

use apl::core::index::PackageIndex;
use std::fs;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let packages_dir = std::env::current_dir()?.join("packages");
    let output_path = std::env::current_dir()?.join("index.bin");

    println!("📦 Updating packages in {}...", packages_dir.display());

    // 1. Iterate over all TOML files
    for entry in fs::read_dir(&packages_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().is_some_and(|e| e == "toml") {
            let pkg_name = path.file_stem().unwrap().to_string_lossy().to_string();
            // In a real implementation, we would fetch upstream API here
            // For now, we just print
            println!("   Checking {}...", pkg_name);
        }
    }

    // 2. Generate Index
    println!("\n📚 Regenerating index at {}...", output_path.display());

    let index = PackageIndex::generate_from_dir(&packages_dir)?;
    index.save_compressed(&output_path)?;

    println!("✅ Done!");

    Ok(())
}
