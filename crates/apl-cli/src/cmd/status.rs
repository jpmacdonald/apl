//! Status command to check for updates and health
use apl_core::paths::apl_home;
use crate::db::StateDb;
use anyhow::{Context, Result};
use apl_schema::index::PackageIndex;
use apl_schema::types::{PackageName, Version};

/// Check status of installed packages
pub fn status() -> Result<()> {
    use crate::ui::Theme;
    use crossterm::style::Stylize;

    let db = StateDb::open().context("Failed to open state database")?;

    // 1. Version
    let pkg_version = env!("APL_VERSION");

    // 2. Index info
    let index_path = apl_home().join("index");
    let index_meta = std::fs::metadata(&index_path).ok();
    let index_date = index_meta
        .and_then(|m| m.modified().ok())
        .map_or_else(
            || "unknown".to_string(),
            |t| {
                chrono::DateTime::<chrono::Local>::from(t)
                    .format("%Y-%m-%d")
                    .to_string()
            },
        );

    let index = if index_path.exists() {
        PackageIndex::load(&index_path).ok()
    } else {
        None
    };

    // 3. Packages and Cache
    let packages = db.list_packages()?;
    let mut total_size: u64 = 0;
    let mut cache_items = 0;

    let cache_dir = apl_home().join("cache");
    if let Ok(entries) = std::fs::read_dir(cache_dir) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                if meta.is_file() {
                    total_size += meta.len();
                    cache_items += 1;
                }
            }
        }
    }

    // 4. Updates
    let mut update_list = Vec::new();
    if let Some(idx) = &index {
        for pkg in &packages {
            if !pkg.active {
                continue;
            }
            let pkg_name = PackageName::new(&pkg.name);
            let pkg_version = Version::from(pkg.version.as_str());

            if let Some(entry) = idx.find(&pkg_name) {
                let latest = match entry.latest() {
                    Some(v) => v.version.clone(),
                    None => continue,
                };
                // Only show update if latest is actually newer (not just different)
                if apl_schema::version::is_newer(pkg_version.as_str(), &latest) {
                    update_list.push((pkg_name, pkg_version, latest));
                }
            }
        }
    }

    // --- RENDER ---
    let theme = Theme::default();
    let label_width = 12;

    println!();
    // U.S. Graphics: Section title, no separators
    println!("{}", "System status".dark_grey());
    println!();

    // Section 1: Core
    println!("{:<width$}{}", "Version:", pkg_version, width = label_width);
    println!(
        "{:<width$}{}",
        "Index:",
        if index.is_some() {
            index_date.to_string()
        } else {
            "Not found".to_string()
        },
        width = label_width
    );
    println!(
        "{:<width$}{} installed",
        "Packages:",
        packages.len(),
        width = label_width
    );
    println!(
        "{:<width$}{} ({} items)",
        "Cache:",
        crate::ui::theme::format_size(total_size),
        cache_items,
        width = label_width
    );

    // Section 2: Updates (if any)
    if update_list.is_empty() {
        println!();
        println!("{}", "System is up to date".dark_grey());
    } else {
        println!();
        println!(
            "{}",
            format!("{} packages can be upgraded", update_list.len()).dark_grey()
        );
        println!();

        for (name, old, new) in update_list {
            let name_part = format!("{:<width$}", name, width = theme.layout.name_width);
            println!(
                "  {} {}  ->  {}",
                name_part.with(theme.colors.package_name),
                old.as_str().dark_grey(),
                new.with(theme.colors.success)
            );
        }
    }

    println!();
    Ok(())
}
