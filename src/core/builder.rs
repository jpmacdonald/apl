//! Build orchestration
//!
//! Runs build scripts inside a Sysroot.

use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

use crate::core::sysroot::Sysroot;

pub struct Builder<'a> {
    sysroot: &'a Sysroot,
}

impl<'a> Builder<'a> {
    pub fn new(sysroot: &'a Sysroot) -> Self {
        Self { sysroot }
    }

    /// Run a build script
    ///
    /// 1. Mounts `source_path` to `/src` in sysroot
    /// 2. Sets up environment (CC, PREFIX)
    /// 3. Runs script in /src
    /// 4. Copies `/usr/local` (or $PREFIX) from sysroot to `output_path`
    pub fn build(
        &self,
        source_path: &Path,
        script: &str,
        output_path: &Path,
        verbose: bool,
        log_path: &Path,
    ) -> Result<()> {
        let sysroot_path = self.sysroot.path().canonicalize()?;

        // 1. Mount Source
        self.sysroot.mount(source_path, Path::new("src"))?;

        // 2. Prepare Destination in Sysroot
        let install_rel = Path::new("usr/local");
        let install_abs = sysroot_path.join(install_rel);
        std::fs::create_dir_all(&install_abs)?;

        // 3. Construct Environment
        let cc = "clang".to_string();
        let cxx = "clang++".to_string();

        // 4. Ensure log directory exists
        if let Some(parent) = log_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // 5. Run Script
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c")
            .arg(script)
            .current_dir(sysroot_path.join("src"))
            .env("CC", &cc)
            .env("CXX", &cxx)
            .env("PREFIX", &install_abs)
            .env("DESTDIR", "")
            .env("JOBS", num_cpus::get().to_string())
            .env("OUTPUT", &install_abs);

        let status = if verbose {
            // Stream to terminal (current behavior)
            cmd.status().context("Failed to execute build script")?
        } else {
            // Redirect to log file
            use std::process::Stdio;
            let log_file =
                std::fs::File::create(log_path).context("Failed to create build log file")?;
            cmd.stdout(Stdio::from(log_file.try_clone()?))
                .stderr(Stdio::from(log_file))
                .status()
                .context("Failed to execute build script")?
        };

        if !status.success() {
            if !verbose {
                // Show last 20 lines from log
                if let Ok(tail) = read_last_lines(log_path, 20) {
                    eprintln!("\nBuild failed. Last 20 lines:");
                    eprintln!("{}", tail);
                    eprintln!("\nFull log: {}", log_path.display());
                }
            }
            anyhow::bail!("Build script failed with exit code: {:?}", status.code());
        }

        // 6. Extract Output
        if output_path.exists() {
            std::fs::remove_dir_all(output_path)?;
        }

        // Rename is atomic and fast if possible
        if std::fs::rename(&install_abs, output_path).is_err() {
            // Fallback to copy
            copy_dir_all(&install_abs, output_path)?;
        }

        Ok(())
    }
}

pub fn copy_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> Result<()> {
    // fs_extra is robust for recursive directory copying
    fs_extra::dir::copy(
        src,
        dst,
        &fs_extra::dir::CopyOptions::new()
            .content_only(true)
            .overwrite(true),
    )
    .map_err(|e| anyhow::anyhow!("Copy failed: {e}"))?;
    Ok(())
}

/// Read the last N lines from a file
fn read_last_lines(path: &Path, n: usize) -> Result<String> {
    use std::fs::File;
    use std::io::{BufRead, BufReader};

    let file = File::open(path)?;
    let reader = BufReader::new(file);

    let lines: Vec<String> = reader.lines().filter_map(|l| l.ok()).collect();

    let start = lines.len().saturating_sub(n);
    Ok(lines[start..].join("\n"))
}
