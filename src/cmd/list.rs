use anyhow::{Context, Result};
use apl::db::StateDb;
use apl::types::{PackageName, Version};
use apl::ui::list::{print_list_footer, print_list_header, print_list_row};

/// List all installed packages
pub fn list() -> Result<()> {
    let db = StateDb::open().context("Failed to open state database")?;
    let packages = db.list_packages()?;
    let mut buffer = apl::ui::buffer::OutputBuffer::default();

    if packages.is_empty() {
        // Use buffer for consistent output even in empty state
        // Or keep println since it's simple info?
        // Let's use standard println for info messages or buffer info?
        // Previous code used println!, let's migrate to a simple info print using buffer equivalent if possible,
        // or just keep println for simple messages but create buffer for future use.
        println!();
        println!("  ℹ No packages installed.");
        println!("  ℹ Run 'apl update && apl install <package>' to get started.");
        return Ok(());
    }

    print_list_header(&mut buffer);

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

        let pkg_name = PackageName::new(&pkg.name);
        let pkg_version = Version::from(pkg.version.as_str());

        print_list_row(&mut buffer, &pkg_name, &pkg_version, pkg_size, &dt, " ");
    }

    print_list_footer(&mut buffer, packages.len(), total_size);

    buffer.flush(); // Ensure output is written

    Ok(())
}
