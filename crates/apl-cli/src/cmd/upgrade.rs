//! Upgrade command - upgrade installed packages to latest versions

use crate::index::PackageIndex;
use anyhow::Result;
use apl_core::paths::apl_home;
use apl_schema::version::is_newer;

/// Upgrade installed packages
pub async fn upgrade(packages: &[String], skip_confirm: bool, dry_run: bool) -> Result<()> {
    use crossterm::style::Stylize;

    let index_path = apl_home().join("index");
    let output = crate::ui::Output::new();

    // Load index
    let Ok(index) = PackageIndex::load(&index_path) else {
        output.error("No index found. Run 'apl update' first.");
        return Ok(());
    };

    // Load installed packages
    let db = crate::db::StateDb::open()?;
    let installed = db.list_packages()?;

    // Determine which packages to upgrade
    let to_upgrade: Vec<_> = if packages.is_empty() {
        // Upgrade all
        installed
            .iter()
            .filter_map(|pkg| {
                if let Some(entry) = index.find(&pkg.name) {
                    let latest = &entry.latest()?.version;
                    // Only upgrade if latest is actually newer (not just different)
                    if is_newer(&pkg.version, latest) {
                        Some((pkg.name.clone(), pkg.version.clone(), latest.clone()))
                    } else {
                        None::<(_, _, _)>
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
                let latest = &entry.latest()?.version;
                // Only upgrade if latest is actually newer (not just different)
                if is_newer(&pkg.version, latest) {
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

    // Actually perform the upgrades by calling install
    let theme = crate::ui::Theme::default();

    println!();

    for (name, old, new) in &to_upgrade {
        let name_col = format!("{:<width$}", name, width = theme.layout.name_width);
        println!(
            "  {} {}  ->  {}",
            name_col.with(theme.colors.package_name),
            old.as_str().dark_grey(),
            new.as_str().with(theme.colors.success)
        );
    }
    println!();

    // Ask for confirmation unless --yes flag is set
    if !skip_confirm {
        use std::io::{self, Write};
        print!("Proceed with upgrade? (Y/n): ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let response = input.trim().to_lowercase();

        if !response.is_empty() && response != "y" && response != "yes" {
            output.info("Upgrade cancelled.");
            return Ok(());
        }
        println!();
    }

    // Extract package names and call install
    let package_names: Vec<String> = to_upgrade.iter().map(|(name, _, _)| name.clone()).collect();

    // Initialize full context for install
    let client = reqwest::Client::builder()
        .tcp_nodelay(true)
        .pool_max_idle_per_host(20)
        .build()?;

    // We already have db and index loaded, reuse them if possible, or recreate handles
    // Since DbHandle is cloneable and we have index, we can construct context.
    // However, existing `db` variable in scope is a `StateDb` (synchronous wrapper from `crate::db::StateDb::open()`?), check imports.
    // Checking imports... `use crate::db::StateDb;` is likely inferred. `db` is declared as `let db = crate::db::StateDb::open()?;`.
    // Wait, `install_packages` needs `DbHandle` (the actor/async one), not `StateDb`.
    // The previous code opened `DbHandle` INSIDE simple `install_packages`.
    // We need to create a `DbHandle` here.

    let db_handle = crate::DbHandle::spawn()?;

    // Output is currently `crate::ui::Output`. Need to wrap in Arc for context.
    // But `Output` might not implement `Reporter` trait? Let's check imports.
    // `output` is used as `reporter` in previous calls, so it implements `Reporter`.
    let reporter = std::sync::Arc::new(output);

    let ctx = crate::ops::Context::new(
        db_handle,
        Some(index),
        client,
        reporter.clone(), // Clone the Arc
    );

    // Call the install logic directly
    crate::ops::install::install_packages(&ctx, &package_names, false)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;

    Ok(())
}
