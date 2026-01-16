//! History command

use anyhow::{Context, Result};
use crate::db::StateDb;
use chrono::DateTime;

pub fn history(pkg_name: &str) -> Result<()> {
    let db = StateDb::open().context("Failed to open state database")?;

    let history = db.get_history(pkg_name)?;

    let output = crate::ui::Output::new();

    if history.is_empty() {
        output.info(&format!("No history found for '{pkg_name}'"));
        return Ok(());
    }

    output.section(&format!("History for '{pkg_name}'"));

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
                    format!("Updated from {from} to {v}")
                } else {
                    format!("Installed {v}")
                }
            }
            "switch" => {
                let from = event.version_from.as_deref().unwrap_or("?");
                let to = event.version_to.as_deref().unwrap_or("?");
                format!("Switched from {from} to {to}")
            }
            "remove" => {
                let from = event.version_from.as_deref().unwrap_or("?");
                format!("Removed {from}")
            }
            _ => format!(
                "{} {}",
                event.action,
                event.version_to.as_deref().unwrap_or("")
            ),
        };

        println!("[{time_str}] {desc}");
    }
    println!();

    Ok(())
}
