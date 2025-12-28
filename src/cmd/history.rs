//! History command

use anyhow::{Context, Result};
use chrono::{DateTime};
use apl::db::StateDb;

pub fn history(pkg_name: &str) -> Result<()> {
    let db = StateDb::open().context("Failed to open state database")?;
    
    let history = db.get_history(pkg_name)?;
    
    if history.is_empty() {
        println!("No history found for '{}'", pkg_name);
        return Ok(());
    }
    
    println!("History for '{}':", pkg_name);
    println!("{}", str::repeat("-", 60));
    
    for event in history {
        // Convert timestamp (millis) to DateTime
        let dt = DateTime::from_timestamp_millis(event.timestamp)
            .unwrap_or_default()
            .with_timezone(&chrono::Local);
            
        let time_str = dt.format("%Y-%m-%d %H:%M:%S").to_string();
        
        // Format action description
        let desc = match event.action.as_str() {
            "install" => {
                let v = event.version_to.as_deref().unwrap_or("?");
                if let Some(from) = event.version_from.as_ref() {
                     format!("Updated from {} to {}", from, v)
                } else {
                     format!("Installed {}", v)
                }
            },
            "switch" => {
                let from = event.version_from.as_deref().unwrap_or("?");
                let to = event.version_to.as_deref().unwrap_or("?");
                format!("Switched from {} to {}", from, to)
            },
            "remove" => {
                let from = event.version_from.as_deref().unwrap_or("?");
                format!("Removed {}", from)
            },
            _ => format!("{} {}", event.action, event.version_to.as_deref().unwrap_or(""))
        };
        
        println!("[{}] {}", time_str, desc);
    }
    println!();
    
    Ok(())
}
