//! Status command to check for updates and health
use anyhow::{Context, Result};
use apl::apl_home;
use apl::db::StateDb;
use apl::index::PackageIndex;

/// Check status of installed packages
pub fn status() -> Result<()> {
    let db = StateDb::open().context("Failed to open state database")?;

    // 1. Version
    let pkg_version = env!("CARGO_PKG_VERSION");

    // 2. Index info
    let index_path = apl_home().join("index.bin");
    let index_meta = std::fs::metadata(&index_path).ok();
    let index_date = index_meta
        .and_then(|m| m.modified().ok())
        .map(|t| {
            chrono::DateTime::<chrono::Local>::from(t)
                .format("%Y-%m-%d")
                .to_string()
        })
        .unwrap_or_else(|| "unknown".to_string());

    let index = if index_path.exists() {
        PackageIndex::load(&index_path).ok()
    } else {
        None
    };

    // 3. Packages and Cache
    let packages = db.list_packages()?;
    let mut total_size: u64 = 0;
    let mut cas_items = 0;

    let cas_dir = apl_home().join("cache");
    if let Ok(entries) = std::fs::read_dir(cas_dir) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                if meta.is_file() {
                    total_size += meta.len();
                    cas_items += 1;
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
            if let Some(entry) = idx.find(&pkg.name) {
                let latest = entry.latest().version.clone();
                if latest != pkg.version {
                    update_list.push((pkg.name.clone(), pkg.version.clone(), latest));
                }
            }
        }
    }

    // --- RENDER ---
    use apl::io::output::format_size;
    use crossterm::style::Stylize;

    println!();
    println!("{}", "APL Package Manager".cyan());
    println!("{}", "─".repeat(40).dark_grey());
    println!();

    println!(
        "  {} {}",
        format!("{:<18}", "Version:").dark_grey(),
        pkg_version.white()
    );
    println!(
        "  {} {}",
        format!("{:<18}", "Index:").dark_grey(),
        format!("{} (up to date)", index_date).green()
    );
    println!(
        "  {} {}",
        format!("{:<18}", "Packages:").dark_grey(),
        format!("{} installed", packages.len()).white()
    );
    println!(
        "  {} {}",
        format!("{:<18}", "Cache:").dark_grey(),
        format!("{} ({} items)", format_size(total_size), cas_items).white()
    );
    println!(
        "  {} {}",
        format!("{:<18}", "Config:").dark_grey(),
        apl_home()
            .join("config.toml")
            .display()
            .to_string()
            .dark_grey()
    );

    if !update_list.is_empty() {
        println!();
        println!(
            "  {} {}",
            format!("{:<18}", "Updates:").dark_grey(),
            format!(
                "{} package{} have updates available",
                update_list.len(),
                if update_list.len() == 1 { "" } else { "s" }
            )
            .yellow()
        );

        for (name, old, new) in update_list {
            println!(
                "                    {} {} {} → {}",
                "└─".dark_grey(),
                name.white(),
                old.dark_grey(),
                new.green()
            );
        }
    }

    println!();
    Ok(())
}
