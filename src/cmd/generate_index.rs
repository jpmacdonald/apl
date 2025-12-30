//! Generate index from package files
//!
//! Scans a directory for .toml files and builds an index.bin

use anyhow::Result;
use apl::index::{IndexBottle, PackageIndex};
use apl::package::PackageType;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn generate_index(packages_dir: &Path, output: &Path) -> Result<()> {
    let mut index = PackageIndex::new();
    index.updated_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    // Read all package files
    if !packages_dir.exists() {
        anyhow::bail!("Packages directory not found: {}", packages_dir.display());
    }

    let output_ui = apl::io::output::CliOutput::new();

    for entry in std::fs::read_dir(packages_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().is_some_and(|ext| ext == "toml") {
            let pkg = apl::package::Package::from_file(&path)?;

            let binaries: Vec<IndexBottle> = pkg
                .binary
                .iter()
                .map(|(arch, binary)| IndexBottle {
                    arch: arch.clone(),
                    url: binary.url.clone(),
                    blake3: binary.blake3.clone(),
                })
                .collect();

            let release = apl::core::index::IndexRelease {
                version: pkg.package.version.clone(),
                bottles: binaries,
                deps: pkg.dependencies.runtime.clone(),
                bin: pkg.install.bin.clone(),
                hints: pkg.hints.post_install.clone(),
                app: pkg.install.app.clone(),
            };

            let type_str = match pkg.package.type_ {
                PackageType::Cli => "cli",
                PackageType::App => "app",
            };

            index.upsert_release(
                &pkg.package.name,
                &pkg.package.description,
                type_str,
                release,
            );

            output_ui.info(&format!(
                "Processed {} (v{})",
                pkg.package.name, pkg.package.version
            ));
        }
    }

    index.save(output)?;
    output_ui.success(&format!(
        "Generated {} with {} packages",
        output.display(),
        index.packages.len()
    ));

    Ok(())
}
