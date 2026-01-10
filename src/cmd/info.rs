//! Info command

use anyhow::{Context, Result, bail};
use apl::apl_home;
use apl::db::StateDb;
use apl::index::PackageIndex;
use apl::types::PackageName;
use apl::ui::theme::format_size;
use crossterm::style::Stylize;

/// Show info about a specific package (U.S. Graphics style)
pub fn info(package_str: &str) -> Result<()> {
    let package = PackageName::new(package_str);
    let db = StateDb::open().context("Failed to open state database")?;

    // Check if installed
    let installed = db.get_package(package.as_str())?;

    // Check index for more info
    let index_path = apl_home().join("index.bin");
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

    // U.S. Graphics style: Package name + version as header
    println!();
    if let Some(entry) = &index_entry {
        let version = entry.latest().map(|v| v.version.as_str()).unwrap_or("?");
        println!(
            "{} {}",
            package.as_str().white().bold(),
            version.dark_grey()
        );
        println!();

        // Description as natural text
        if !entry.description.is_empty() {
            println!("{}", entry.description);
        }
        println!();

        // Key-value pairs with consistent alignment
        let label_width = 12;
        if !entry.homepage.is_empty() {
            println!(
                "{:<width$}{}",
                "Homepage:",
                entry.homepage,
                width = label_width
            );
        }
        if let Some(latest) = entry.latest() {
            if !latest.deps.is_empty() {
                println!(
                    "{:<width$}{}",
                    "Requires:",
                    latest.deps.join(", "),
                    width = label_width
                );
            }
        }

        // Installed status
        if let Some(pkg) = &installed {
            let size_str = format_size(pkg.size_bytes);
            println!(
                "{:<width$}Yes ({})",
                "Installed:",
                size_str,
                width = label_width
            );
        } else {
            println!("{:<width$}No", "Installed:", width = label_width);
        }
    } else if let Some(pkg) = &installed {
        // Only installed, no index entry
        println!(
            "{} {}",
            package.as_str().white().bold(),
            pkg.version.as_str().dark_grey()
        );
        println!();
        let size_str = format_size(pkg.size_bytes);
        println!("{:<width$}Yes ({})", "Installed:", size_str, width = 12);
    }

    Ok(())
}
