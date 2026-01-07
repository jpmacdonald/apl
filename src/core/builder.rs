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
    /// 2. Mounts all `deps` to `/deps/{name}`
    /// 3. Sets up environment (CC, PREFIX, CPATH, etc.)
    /// 4. Runs script in /src
    /// 5. Copies `/usr/local` (or $PREFIX) from sysroot to `output_path`
    pub fn build(
        &self,
        source_path: &Path,
        deps: &[(String, std::path::PathBuf)],
        script: &str,
        output_path: &Path,
        verbose: bool,
        log_path: &Path,
    ) -> Result<()> {
        let sysroot_path = self.sysroot.path().canonicalize()?;

        // 1. Mount Source
        self.sysroot.mount(source_path, Path::new("src"))?;

        // 2. Mount Dependencies and build environment paths
        let mut cpath = Vec::new();
        let mut library_path = Vec::new();
        let mut pkg_config_path = Vec::new();

        for (name, dep_path) in deps {
            let target_rel = Path::new("deps").join(name);
            self.sysroot.mount(dep_path, &target_rel)?;

            let abs_dep = sysroot_path.join(&target_rel);
            if abs_dep.join("include").exists() {
                cpath.push(abs_dep.join("include").to_string_lossy().to_string());
            }
            if abs_dep.join("lib").exists() {
                library_path.push(abs_dep.join("lib").to_string_lossy().to_string());
            }
            if abs_dep.join("lib/pkgconfig").exists() {
                pkg_config_path.push(abs_dep.join("lib/pkgconfig").to_string_lossy().to_string());
            }
        }

        // 3. Prepare Destination in Sysroot
        let install_rel = Path::new("usr/local");
        let install_abs = sysroot_path.join(install_rel);
        std::fs::create_dir_all(&install_abs)?;

        // 4. Construct Environment
        let cc = "clang".to_string();
        let cxx = "clang++".to_string();

        // 5. Ensure log directory exists
        if let Some(parent) = log_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // 6. Run Script
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

        // Add dependency search paths
        if !cpath.is_empty() {
            cmd.env("CPATH", cpath.join(":"));
            cmd.env("C_INCLUDE_PATH", cpath.join(":"));
            cmd.env("CPLUS_INCLUDE_PATH", cpath.join(":"));
        }
        if !library_path.is_empty() {
            cmd.env("LIBRARY_PATH", library_path.join(":"));
            // For runtime checks during some builds
            cmd.env("DYLD_LIBRARY_PATH", library_path.join(":"));
        }
        if !pkg_config_path.is_empty() {
            cmd.env("PKG_CONFIG_PATH", pkg_config_path.join(":"));
        }

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
                    eprintln!("{tail}");
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

    let lines: Vec<String> = reader.lines().map_while(Result::ok).collect();

    let start = lines.len().saturating_sub(n);
    Ok(lines[start..].join("\n"))
}
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_builder_env_setup() {
        let sysroot = Sysroot::new().unwrap();
        let builder = Builder::new(&sysroot);

        let src_tmp = tempdir().unwrap();
        let out_tmp = tempdir().unwrap();
        let log_tmp = tempdir().unwrap().path().join("build.log");

        // Create a fake dependency with include/lib
        let dep_tmp = tempdir().unwrap();
        std::fs::create_dir_all(dep_tmp.path().join("include")).unwrap();
        std::fs::create_dir_all(dep_tmp.path().join("lib")).unwrap();
        std::fs::write(dep_tmp.path().join("include/header.h"), "").unwrap();

        let deps = vec![("test-dep".to_string(), dep_tmp.path().to_path_buf())];

        // We can't easily run a full build in a test without a real shell and tools,
        // but we can verify the directory structure and paths.
        let sysroot_path = sysroot.path().canonicalize().unwrap();

        // Mock a successful build by creating the output file in sysroot
        // this simulates what the script would do.
        let _ = builder.build(
            src_tmp.path(),
            &deps,
            "mkdir -p $PREFIX/bin && touch $PREFIX/bin/test-bin",
            out_tmp.path(),
            false,
            &log_tmp,
        );

        // Verify that dependency was mounted correctly
        let mounted_dep = sysroot_path.join("deps/test-dep");
        assert!(mounted_dep.exists());
        assert!(mounted_dep.join("include/header.h").exists());

        // Verify that output was extracted
        assert!(out_tmp.path().join("bin/test-bin").exists());
    }
}
