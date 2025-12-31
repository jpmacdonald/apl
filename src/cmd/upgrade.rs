//! Upgrade command - upgrade installed packages to latest versions

use anyhow::Result;
use apl::apl_home;
use apl::index::PackageIndex;

/// Upgrade installed packages
pub async fn upgrade(packages: &[String], dry_run: bool) -> Result<()> {
    let index_path = apl_home().join("index.bin");
    let output = apl::ui::Output::new();

    // Load index
    let index = match PackageIndex::load(&index_path) {
        Ok(idx) => idx,
        Err(_) => {
            output.error("No index found. Run 'apl update' first.");
            return Ok(());
        }
    };

    // Load installed packages
    let db = apl::db::StateDb::open()?;
    let installed = db.list_packages()?;

    // Determine which packages to upgrade
    let to_upgrade: Vec<_> = if packages.is_empty() {
        // Upgrade all
        installed
            .iter()
            .filter_map(|pkg| {
                if let Some(entry) = index.find(&pkg.name) {
                    let latest = &entry.latest().version;
                    if latest != &pkg.version {
                        Some((pkg.name.clone(), pkg.version.clone(), latest.clone()))
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect()
    } else {
        // Upgrade specific packages
        packages
            .iter()
            .filter_map(|name| {
                let pkg = installed.iter().find(|p| &p.name == name)?;
                let entry = index.find(name)?;
                let latest = &entry.latest().version;
                if latest != &pkg.version {
                    Some((pkg.name.clone(), pkg.version.clone(), latest.clone()))
                } else {
                    None
                }
            })
            .collect()
    };

    if to_upgrade.is_empty() {
        output.success("All packages are up to date.");
        return Ok(());
    }

    if dry_run {
        output.info("Would upgrade:");
        for (name, old, new) in &to_upgrade {
            output.info(&format!("  {name}: {old} -> {new}"));
        }
        return Ok(());
    }

    // For now, show what would be upgraded
    // Full implementation would call install logic for each package
    // For now, show what would be upgraded
    // Full implementation would call install logic for each package
    use crossterm::style::Stylize;
    let theme = apl::ui::Theme::default();

    println!();
    println!(
        "   {} {} {}",
        format!("{:<width$}", "PACKAGE", width = theme.layout.name_width).dark_grey(),
        format!("{:<width$}", "UPDATE", width = 25).dark_grey(),
        "STATUS".dark_grey()
    );
    println!("{}", "─".repeat(theme.layout.table_width).dark_grey());

    for (name, old, new) in &to_upgrade {
        let name_col = format!("{:<width$}", name, width = theme.layout.name_width);
        let version_col = format!("{:<width$}", format!("{} → {}", old, new), width = 25);

        println!(
            "   {} {} {}",
            name_col.with(theme.colors.package_name),
            version_col.with(theme.colors.version),
            "available".with(theme.colors.warning)
        );
    }
    println!("{}", "─".repeat(theme.layout.table_width).dark_grey());

    println!();
    println!("   Run 'apl install <package>' to upgrade individually.");
    println!();

    Ok(())
}
