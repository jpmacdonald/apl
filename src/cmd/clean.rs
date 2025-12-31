//! Clean command (garbage collection)

use anyhow::Result;
use apl::ui::Output;

/// Garbage collect orphaned files
pub fn clean(_dry_run: bool) -> Result<()> {
    let output = Output::new();

    // Legacy cache cleanup
    let cache_dir = apl::try_apl_home().map(|h| h.join("cache"));
    if let Some(dir) = cache_dir {
        if dir.exists() {
            output.info("Cleaning cache directory...");
            if !_dry_run {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }

    output.success("System is clean.");
    Ok(())
}
