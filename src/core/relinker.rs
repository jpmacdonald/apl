//! Mach-O Relinker using install_name_tool
//!
//! Ensures binaries are relocatable and use relative paths for libraries.
//! Strategy:
//! 1. Binaries: -add_rpath @executable_path/../lib
//! 2. Dylibs: -id @rpath/libname.dylib

use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

/// Relinker for Mach-O binaries/dylibs
pub struct Relinker;

impl Relinker {
    /// Fix up a binary to look for libraries in ../lib
    pub fn fix_binary(binary_path: &Path) -> Result<()> {
        Self::run_install_name_tool(binary_path, &["-add_rpath", "@executable_path/../lib"])
    }

    /// Fix up a dylib to have an @rpath ID
    pub fn fix_dylib(dylib_path: &Path) -> Result<()> {
        let name = dylib_path
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("Invalid dylib path"))?
            .to_string_lossy();

        let new_id = format!("@rpath/{name}");

        Self::run_install_name_tool(dylib_path, &["-id", &new_id])
    }

    /// Change a dependency path in a binary/dylib
    /// e.g. /usr/local/lib/libssl.dylib -> @rpath/libssl.dylib
    pub fn change_dep(path: &Path, old: &str, new: &str) -> Result<()> {
        Self::run_install_name_tool(path, &["-change", old, new])
    }

    /// Helper to run install_name_tool
    fn run_install_name_tool(path: &Path, args: &[&str]) -> Result<()> {
        let output = Command::new("install_name_tool")
            .args(args)
            .arg(path)
            .output()
            .context("Failed to spawn install_name_tool")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("install_name_tool failed on {}: {}", path.display(), stderr);
        }

        Ok(())
    }
}
