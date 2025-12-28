//! List command

use anyhow::{Context, Result};
use apl::db::StateDb;

/// List all installed packages
pub fn list() -> Result<()> {
    let db = StateDb::open().context("Failed to open state database")?;
    
    let packages = db.list_packages()?;
    
    if packages.is_empty() {
        println!("No packages installed.");
        println!("Run 'dl update && dl install <package>' to get started.");
        return Ok(());
    }
    
    println!("📋 Installed packages:");
    for pkg in packages {
        let ago = format_relative_time(pkg.installed_at);
        println!("  {} {} (installed {})", pkg.name, pkg.version, ago);
    }
    
    Ok(())
}

/// Format a timestamp as relative time
fn format_relative_time(unix_timestamp: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    
    let diff = now - unix_timestamp;
    
    if diff < 60 {
        "just now".to_string()
    } else if diff < 3600 {
        format!("{} minutes ago", diff / 60)
    } else if diff < 86400 {
        format!("{} hours ago", diff / 3600)
    } else {
        format!("{} days ago", diff / 86400)
    }
}
