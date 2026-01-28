//! Info command

use anyhow::{Context, Result, bail};
use apl_core::paths::apl_home;
use crate::db::StateDb;
use crate::index::PackageIndex;
use apl_schema::types::PackageName;
use crate::ui::theme::format_size;
use crossterm::style::Stylize;

/// Show info about a specific package
pub fn info(package_str: &str) -> Result<()> {
    let package = PackageName::new(package_str);
    let db = StateDb::open().context("Failed to open state database")?;

    let installed = db.get_package(package.as_str())?;

    let index_path = apl_home().join("index");
    let index_entry = if index_path.exists() {
        PackageIndex::load(&index_path)
            .ok()
            .and_then(|idx| idx.find(&package).cloned())
    } else {
        None
    };

    if installed.is_none() && index_entry.is_none() {
        bail!("Package '{package}' not found");
    }

    let lw = 12;

    println!();
    if let Some(entry) = &index_entry {
        let version = entry.latest().map_or("?", |v| v.version.as_str());
        println!(
            "  {} {}",
            package.as_str().white().bold(),
            version.dark_grey()
        );

        if !entry.description.is_empty() {
            println!("  {}", entry.description);
        }
        println!();

        if !entry.homepage.is_empty() {
            println!("  {:<lw$}{}", "homepage", entry.homepage);
        }
        if let Some(latest) = entry.latest() {
            if !latest.deps.is_empty() {
                println!("  {:<lw$}{}", "requires", latest.deps.join(", "));
            }
        }

        if let Some(pkg) = &installed {
            let size_str = format_size(pkg.size_bytes);
            let dt = chrono::DateTime::from_timestamp(pkg.installed_at, 0)
                .unwrap_or_default()
                .format("%Y-%m-%d")
                .to_string();
            println!("  {:<lw$}{}, {}", "installed", size_str, dt);
        }
    } else if let Some(pkg) = &installed {
        println!(
            "  {} {}",
            package.as_str().white().bold(),
            pkg.version.as_str().dark_grey()
        );
        println!();
        let size_str = format_size(pkg.size_bytes);
        let dt = chrono::DateTime::from_timestamp(pkg.installed_at, 0)
            .unwrap_or_default()
            .format("%Y-%m-%d")
            .to_string();
        println!("  {:<lw$}{}, {}", "installed", size_str, dt);
    }

    Ok(())
}
