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
    pub fn build(&self, source_path: &Path, script: &str, output_path: &Path) -> Result<()> {
        let sysroot_path = self.sysroot.path().canonicalize()?;

        // 1. Mount Source
        self.sysroot.mount(source_path, Path::new("src"))?;

        // 2. Prepare Destination in Sysroot
        // We assume standard usage installs to /usr/local or /opt/apl
        // Let's define prefix as /usr/local for now
        let install_rel = Path::new("usr/local");
        let install_abs = sysroot_path.join(install_rel);
        std::fs::create_dir_all(&install_abs)?;

        // 3. Construct Environment
        // Use host compiler directly (Phase 1 simplification)
        // Full sysroot isolation with copied SDK comes in Phase 2
        let cc = "clang".to_string();
        let cxx = "clang++".to_string();

        // 4. Run Script
        // We run /bin/sh from the host.
        let status = Command::new("/bin/sh")
            .arg("-c")
            .arg(script)
            .current_dir(sysroot_path.join("src"))
            .env("CC", &cc)
            .env("CXX", &cxx)
            .env("PREFIX", &install_abs)
            // Usually build scripts expect PREFIX to be where they write TO.
            // If we give them "/usr/local", they write to /usr/local.
            // But we are running on host, just changed directory.
            // Wait, if we are NOT chrooting, "/usr/local" means THE HOST /usr/local.
            // DANGER!
            // We are not chrooting because we need host tools (make, git, cargo).
            // Implementation Plan said: "--sysroot".
            // Clonefile Sysroot is for *compile inputs*.
            // *Install outputs* must be directed to a safe place.
            //
            // We must set PREFIX to the *absolute path inside the sysroot*.
            // i.e. /tmp/apl-build-xyz/usr/local
            .env("PREFIX", &install_abs)
            .env("DESTDIR", "") // Explicitly empty to avoid confusion
            .env("JOBS", num_cpus::get().to_string())
            .env("OUTPUT", &install_abs) // Alias for clearer scripts
            .status()
            .context("Failed to execute build script")?;

        if !status.success() {
            anyhow::bail!("Build script failed with exit code: {:?}", status.code());
        }

        // 5. Extract Output
        // Copy `install_abs` to `output_path`
        // We can use simple recursive copy or rename if on same volume?
        // `output_path` is usually `~/.apl/store/pkg-v`.
        // `install_abs` is `/tmp/...`.
        // Rename might work if /tmp is same volume. MacOS /tmp is often separate?
        // Let's use recursive copy to be safe.
        // Actually, we can assume standardized FS moves later.

        if output_path.exists() {
            std::fs::remove_dir_all(output_path)?;
        }

        // Rename is atomic and fast if possible.
        if std::fs::rename(&install_abs, output_path).is_err() {
            // Fallback to copy
            copy_dir_all(&install_abs, output_path)?;
        }

        Ok(())
    }
}

fn copy_dir_all(src: impl AsRef<Path>, dst: impl AsRef<Path>) -> Result<()> {
    fs_extra::dir::copy(
        src,
        dst,
        &fs_extra::dir::CopyOptions::new().content_only(true),
    )
    .map_err(|e| anyhow::anyhow!("Copy failed: {e}"))?;
    Ok(())
}
