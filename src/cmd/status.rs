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
            if let Some(entry) = idx.find(&pkg.name) {
                let latest = entry.latest().version.clone();
                // Only show update if latest is actually newer (not just different)
                if apl::core::version::is_newer(&pkg.version, &latest) {
                    update_list.push((pkg.name.clone(), pkg.version.clone(), latest));
                }
            }
        }
    }

    // --- RENDER ---
    use apl::ui::Theme;
    use crossterm::style::Stylize;

    let theme = Theme::default();

    println!();
    // Header
    println!("   {}", "System Status".with(theme.colors.header));
    println!("{}", "─".repeat(theme.layout.table_width).dark_grey());

    // Section 1: Core
    println!(
        "   {:<18} {}",
        "Version:".with(theme.colors.secondary),
        pkg_version.with(theme.colors.version)
    );
    println!(
        "   {:<18} {}",
        "Index:".with(theme.colors.secondary),
        if index.is_some() {
            format!("{} (up to date)", index_date).with(theme.colors.success)
        } else {
            "Not found".to_string().with(theme.colors.error)
        }
    );
    println!(
        "   {:<18} {}",
        "Packages:".with(theme.colors.secondary),
        format!("{} installed", packages.len()).with(theme.colors.version)
    );
    println!(
        "   {:<18} {}",
        "Cache:".with(theme.colors.secondary),
        format!(
            "{} ({} items)",
            apl::ui::theme::format_size(total_size),
            cache_items
        )
        .with(theme.colors.version)
    );
    println!(
        "   {:<18} {}",
        "Config:".with(theme.colors.secondary),
        apl_home()
            .join("config.toml")
            .display()
            .to_string()
            .with(theme.colors.secondary)
    );

    // Section 2: Updates (if any)
    if !update_list.is_empty() {
        println!();
        println!(
            "   {:<18} {}",
            "Updates:".with(theme.colors.secondary),
            format!(
                "{} package{} available",
                update_list.len(),
                if update_list.len() == 1 { "" } else { "s" }
            )
            .with(theme.colors.warning)
        );

        for (name, old, new) in update_list {
            println!(
                "     {} {} {} → {}",
                "└─".dark_grey(),
                name.with(theme.colors.package_name),
                old.dark_grey(),
                new.with(theme.colors.success)
            );
        }
    }

    println!("{}", "─".repeat(theme.layout.table_width).dark_grey());
    println!();
    Ok(())
}
