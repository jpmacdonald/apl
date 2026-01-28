//! Clean command (garbage collection)

use crate::ui::Output;
use anyhow::Result;

/// Garbage collect orphaned files
pub fn clean(dry_run: bool) -> Result<()> {
    let output = Output::new();

    // Legacy cache cleanup
    let cache_dir = crate::try_apl_home().map(|h| h.join("cache"));
    if let Some(dir) = cache_dir {
        if dir.exists() {
            output.info("Cleaning cache directory...");
            if !dry_run {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }

    output.success("System is clean.");
    Ok(())
}
