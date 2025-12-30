//! Clean command (garbage collection)

use anyhow::Result;
use apl::io::output::CliOutput;

/// Garbage collect orphaned files
pub fn clean(_dry_run: bool) -> Result<()> {
    let output = CliOutput::new();

    // Legacy CAS cleanup
    let cas_dir = apl::try_apl_home().map(|h| h.join("cache"));
    if let Some(dir) = cas_dir {
        if dir.exists() {
            output.info("Removing legacy CAS cache directory...");
            if !_dry_run {
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }

    output.success("System is clean.");
    Ok(())
}
