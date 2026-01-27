//! Mach-O path patching utility.
//!
//! Patches rpaths and load commands to ensure binaries function portably.
//! Implementation Details:
//! 1. Binaries: -add_rpath @executable_path/../lib
//! 2. Dylibs: -id @rpath/libname.dylib

use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

/// Utilities for patching Mach-O headers.
///
/// # Implementation Note: Mach-O and `RPaths`
/// macOS binaries (Mach-O) look for shared libraries (dylibs) using "load commands" embedded in the file header.
/// Unlike ELF (Linux) which uses `LD_LIBRARY_PATH` or `rpath`, macOS relies heavily on the `@rpath` token.
///
/// - **`@rpath`**: A variable placeholder in a dylib's ID (e.g., `@rpath/libssl.dylib`).
/// - **`LC_RPATH`**: A load command in the *executable* that defines values for `@rpath` (e.g., `@executable_path/../lib`).
///
/// By setting the Dylib ID to start with `@rpath/` and adding a relative `LC_RPATH` to the binary,
/// we make the package **relocatable**. You can move the entire directory structure anywhere, and the
/// binary will still find its libraries in `../lib` relative to itself.
#[derive(Debug)]
pub struct Relinker;

impl Relinker {
    /// Adds a relative `../lib` rpath to an executable via `install_name_tool`.
    ///
    /// After patching, the binary is re-signed with an ad-hoc signature.
    ///
    /// # Errors
    ///
    /// Returns an error if `install_name_tool` is not found, exits with a
    /// non-zero status, or the subsequent code-signing step fails.
    pub fn fix_binary(binary_path: &Path) -> Result<()> {
        Self::run_install_name_tool(binary_path, &["-add_rpath", "@executable_path/../lib"])?;
        Self::resign(binary_path)
    }

    /// Sets the install ID of a dynamic library to `@rpath/<filename>`.
    ///
    /// After patching, the library is re-signed with an ad-hoc signature.
    ///
    /// # Errors
    ///
    /// Returns an error if the path has no filename, `install_name_tool`
    /// is not found or fails, or the subsequent code-signing step fails.
    pub fn fix_dylib(dylib_path: &Path) -> Result<()> {
        let name = dylib_path
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("Invalid dylib path"))?
            .to_string_lossy();

        let new_id = format!("@rpath/{name}");

        Self::run_install_name_tool(dylib_path, &["-id", &new_id])?;
        Self::resign(dylib_path)
    }

    /// Updates a load command to point to a new location.
    ///
    /// For example, `/usr/local/lib/libssl.dylib` can be changed to
    /// `@rpath/libssl.dylib`.
    ///
    /// # Errors
    ///
    /// Returns an error if `install_name_tool` is not found, exits with a
    /// non-zero status, or the subsequent code-signing step fails.
    pub fn change_dep(path: &Path, old: &str, new: &str) -> Result<()> {
        Self::run_install_name_tool(path, &["-change", old, new])?;
        Self::resign(path)
    }

    /// Executes `install_name_tool` and handles errors.
    fn run_install_name_tool(path: &Path, args: &[&str]) -> Result<()> {
        let output = match Command::new("install_name_tool")
            .args(args)
            .arg(path)
            .output()
        {
            Ok(o) => o,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                anyhow::bail!(
                    "'install_name_tool' not found. Please install Xcode Command Line Tools: xcode-select --install"
                );
            }
            Err(e) => return Err(e).context("Failed to spawn install_name_tool"),
        };

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("install_name_tool failed on {}: {}", path.display(), stderr);
        }

        Ok(())
    }

    /// Re-applies ad-hoc code signing to patched Mach-O binaries.
    ///
    /// Uses `codesign -s - --force` to apply an ad-hoc signature while
    /// preserving existing entitlements, requirements, flags, and runtime
    /// metadata.
    ///
    /// # Errors
    ///
    /// Returns an error if the `codesign` process cannot be spawned.
    pub fn resign(path: &Path) -> Result<()> {
        let _ = Command::new("codesign")
            .args([
                "-s",
                "-",
                "--force",
                "--preserve-metadata=entitlements,requirements,flags,runtime",
            ])
            .arg(path)
            .output()
            .context("Failed to spawn codesign");

        Ok(())
    }

    /// Recursively scan a directory and relink all Mach-O files.
    ///
    /// - Files in `bin/` are treated as executables.
    /// - Files ending in `.dylib`, `.so`, or in `lib/` are treated as libraries.
    ///
    /// # Errors
    ///
    /// Returns an error if directory traversal fails or if `install_name_tool` fails.
    pub fn relink_all(root: &Path) -> Result<()> {
        for entry in walkdir::WalkDir::new(root)
            .into_iter()
            .filter_map(std::result::Result::ok)
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.path();
            let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let parent_name = path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("");

            if !Self::is_macho(path) {
                continue;
            }

            if parent_name == "bin" {
                Self::fix_binary(path)?;
            } else if parent_name == "lib"
                || path.extension().is_some_and(|ext| {
                    ext.eq_ignore_ascii_case("dylib") || ext.eq_ignore_ascii_case("so")
                })
                || file_name.contains(".so.")
            {
                Self::fix_dylib(path)?;
            }
        }
        Ok(())
    }

    /// Checks if a file is a Mach-O binary (magic bytes).
    fn is_macho(path: &Path) -> bool {
        use std::io::Read;
        let mut f = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(_) => return false,
        };
        let mut magic = [0u8; 4];
        if f.read_exact(&mut magic).is_err() {
            return false;
        }
        // feedface, feedfacf, cafebabe (universal) - and their LE/BE variants
        matches!(
            magic,
            [0xfe, 0xed, 0xfa, 0xce]
                | [0xfe, 0xed, 0xfa, 0xcf]
                | [0xcf, 0xfa, 0xed, 0xfe]
                | [0xce, 0xfa, 0xed, 0xfe]
                | [0xca, 0xfe, 0xba, 0xbe]
        )
    }
}
