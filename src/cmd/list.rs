//! List command

use anyhow::{Context, Result};
use apl::db::StateDb;
use apl::io::output::{print_list_footer, print_list_header, print_list_row};

/// List all installed packages
pub fn list() -> Result<()> {
    let db = StateDb::open().context("Failed to open state database")?;
    let packages = db.list_packages()?;

    if packages.is_empty() {
        println!();
        println!("  ℹ No packages installed.");
        println!("  ℹ Run 'apl update && apl install <package>' to get started.");
        return Ok(());
    }

    print_list_header();

    let mut total_size: u64 = 0;

    for pkg in &packages {
        // Use stored size
        let pkg_size = pkg.size_bytes;
        total_size += pkg_size;

        // Format installed date
        let dt = chrono::DateTime::from_timestamp(pkg.installed_at, 0)
            .unwrap_or_default()
            .format("%Y-%m-%d")
            .to_string();

        print_list_row(&pkg.name, &pkg.version, pkg_size, &dt, " ");
    }

    print_list_footer(packages.len(), total_size);

    Ok(())
}
