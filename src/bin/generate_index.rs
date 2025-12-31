//! Tool to regenerate the package index
//! Usage: cargo run --bin generate_index

use apl::core::index::PackageIndex;

fn main() -> anyhow::Result<()> {
    let packages_dir = std::env::current_dir()?.join("packages");
    let output_path = std::env::current_dir()?.join("index.bin");

    println!("📚 Regenerating index from {}...", packages_dir.display());

    let index = PackageIndex::generate_from_dir(&packages_dir)?;
    index.save_compressed(&output_path)?;

    println!("✅ Index generated at {}", output_path.display());
    Ok(())
}
