//! Generate index from formula files
//!
//! Scans a directory for .toml files and builds an index.bin

use anyhow::Result;
use dl::index::{PackageIndex, IndexBottle};
use dl::formula::PackageType;
use std::time::{SystemTime, UNIX_EPOCH};
use std::path::Path;

pub fn generate_index(formulas_dir: &Path, output: &Path) -> Result<()> {
    
    let mut index = PackageIndex::new();
    index.updated_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    
    // Read all formula files
    if !formulas_dir.exists() {
        // Return clear error if dir doesn't exist
        anyhow::bail!("Formulas directory not found: {}", formulas_dir.display());
    }

    for entry in std::fs::read_dir(formulas_dir)? {
        let entry = entry?;
        let path = entry.path();
        
        if path.extension().is_some_and(|ext| ext == "toml") {
            let formula = dl::formula::Formula::from_file(&path)?;
            
            let bottles: Vec<IndexBottle> = formula.bottle.iter()
                .map(|(arch, bottle)| IndexBottle {
                    arch: arch.clone(),
                    url: bottle.url.clone(),
                    blake3: bottle.blake3.clone(),
                })
                .collect();
            
            let release = dl::core::index::IndexRelease {
                version: formula.package.version.clone(),
                bottles,
                deps: formula.dependencies.runtime.clone(),
                bin: formula.install.bin.clone(),
                hints: formula.hints.post_install.clone(),
                app: formula.install.app.clone(),
            };
            
            let type_str = match formula.package.type_ {
                PackageType::Cli => "cli",
                PackageType::App => "app",
            };
            
            index.upsert_release(
                &formula.package.name,
                &formula.package.description,
                type_str,
                release
            );
            
            println!("  + {} (v{})", formula.package.name, formula.package.version);
        }
    }
    
    index.save(output)?;
    println!("✓ Generated {} with {} packages", output.display(), index.packages.len());
    
    Ok(())
}
