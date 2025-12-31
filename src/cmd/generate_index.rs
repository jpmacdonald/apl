//! Generate index from package files
//!
//! Scans a directory for .toml files and builds an index.bin

use anyhow::Result;
use apl::index::{IndexBinary, PackageIndex, VersionInfo};
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

    let output_ui = apl::ui::Output::new();

    for entry in std::fs::read_dir(packages_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().is_some_and(|ext| ext == "toml") {
            let pkg = apl::package::Package::from_file(&path)?;

            let binaries: Vec<IndexBinary> = pkg
                .binary
                .iter()
                .map(|(arch, binary)| IndexBinary {
                    arch: arch.clone(),
                    url: binary.url.clone(),
                    blake3: binary.blake3.clone(),
                })
                .collect();

            let release = VersionInfo {
                version: pkg.package.version.clone(),
                binaries,
                deps: pkg.dependencies.runtime.clone(),
                build_deps: pkg.dependencies.build.clone(),
                build_script: pkg
                    .build
                    .as_ref()
                    .map(|b| b.script.clone())
                    .unwrap_or_default(),
                bin: pkg.install.bin.clone(),
                hints: pkg.hints.post_install.clone(),
                app: pkg.install.app.clone(),
                source: Some(apl::index::IndexSource {
                    url: pkg.source.url.clone(),
                    blake3: pkg.source.blake3.clone(),
                }),
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

    index.save_compressed(output)?;
    output_ui.success(&format!(
        "Generated {} with {} packages",
        output.display(),
        index.packages.len()
    ));

    Ok(())
}
