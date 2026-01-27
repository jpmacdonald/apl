//! Build orchestration for hermetic, reproducible builds.
//!
//! Executes build scripts inside an APFS copy-on-write [`Sysroot`], ensuring
//! complete isolation from the host environment. Every build runs with a
//! sanitised environment: host variables are cleared, and only the minimal
//! set required for compilation is injected. This prevents "works on my
//! machine" failures and ensures binary reproducibility across CI and
//! developer machines.
//!
//! ## Environment contract
//!
//! Build scripts receive exactly these variables (nothing more):
//!
//! | Variable | Value |
//! |---|---|
//! | `PATH` | `/usr/bin:/bin:/usr/sbin:/sbin` (Xcode CLT added when present) |
//! | `HOME` | Sysroot root (prevents reading host dotfiles) |
//! | `TERM` | `dumb` |
//! | `LANG` | `en_US.UTF-8` |
//! | `CC` / `CXX` | `clang` / `clang++` |
//! | `ARCH` | Target architecture (`aarch64` or `x86_64`) |
//! | `PREFIX` | Install destination inside sysroot |
//! | `OUTPUT` | Same as `PREFIX` |
//! | `DESTDIR` | Empty string |
//! | `JOBS` | Logical CPU count |
//! | `DEPS_DIR` | Absolute path to the mounted dependencies root |
//! | `DEP_<NAME>` | Per-dependency absolute path (name uppercased, hyphens to underscores) |
//! | `CFLAGS` / `CPPFLAGS` | `-I` flags for each dependency's `include/` |
//! | `LDFLAGS` | `-L` flags for each dependency's `lib/` |
//! | `CPATH`, `C_INCLUDE_PATH`, `CPLUS_INCLUDE_PATH` | Colon-separated include paths |
//! | `LIBRARY_PATH`, `DYLD_LIBRARY_PATH` | Colon-separated library paths |
//! | `PKG_CONFIG_PATH` | Colon-separated `lib/pkgconfig` paths |
//! | `MACOSX_DEPLOYMENT_TARGET` | `13.0` (Ventura baseline) |
//! | `SOURCE_DATE_EPOCH` | `0` (epoch zero for reproducible timestamps) |

use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;

use crate::sysroot::Sysroot;

/// Minimum macOS version we target. Ventura (13.0) is the baseline: it is the
/// oldest release that ships with the required APFS clonefile(2) semantics and
/// a Rosetta 2 translation layer for x86_64 binaries on Apple Silicon.
const MACOSX_DEPLOYMENT_TARGET: &str = "13.0";

/// Fixed epoch for `SOURCE_DATE_EPOCH`. Using zero (1970-01-01T00:00:00Z)
/// ensures that all embedded timestamps are identical across builds regardless
/// of when the build actually ran.
const SOURCE_DATE_EPOCH: &str = "0";

/// Orchestrates hermetic package builds inside an APFS [`Sysroot`].
///
/// See the [module-level documentation](self) for the full environment
/// contract.
#[derive(Debug)]
pub struct Builder<'a> {
    sysroot: &'a Sysroot,
}

impl<'a> Builder<'a> {
    /// Create a new builder backed by the given [`Sysroot`].
    pub fn new(sysroot: &'a Sysroot) -> Self {
        Self { sysroot }
    }

    /// Execute a build script inside the sysroot.
    ///
    /// 1. Mounts `source_path` at `<sysroot>/src`.
    /// 2. Mounts each dependency at `<sysroot>/deps/<name>`.
    /// 3. Constructs a fully hermetic environment (see module docs).
    /// 4. Runs `script` via `/bin/sh -c` with cwd = `<sysroot>/src`.
    /// 5. Moves `<sysroot>/usr/local` (the `$PREFIX`) to `output_path`.
    ///
    /// # Errors
    ///
    /// Returns an error if any mount operation fails, the build script
    /// exits with a non-zero status, or the output cannot be moved to
    /// `output_path`.
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

        // -- 1. Mount source tree ------------------------------------------
        self.sysroot.mount(source_path, Path::new("src"))?;

        // -- 2. Mount dependencies & collect search paths -------------------
        let deps_dir = sysroot_path.join("deps");
        let mut include_paths: Vec<String> = Vec::new();
        let mut library_paths: Vec<String> = Vec::new();
        let mut pkg_config_paths: Vec<String> = Vec::new();
        let mut cflags: Vec<String> = Vec::new();
        let mut ldflags: Vec<String> = Vec::new();
        let mut per_dep_env: Vec<(String, String)> = Vec::new();

        for (name, dep_path) in deps {
            let target_rel = Path::new("deps").join(name);
            self.sysroot.mount(dep_path, &target_rel)?;

            let abs_dep = sysroot_path.join(&target_rel);

            // Per-dependency env var: DEP_OPENSSL, DEP_ZLIB, etc.
            let env_name = format!(
                "DEP_{}",
                name.to_uppercase().replace(['-', '.'], "_")
            );
            per_dep_env.push((env_name, abs_dep.to_string_lossy().to_string()));

            if abs_dep.join("include").exists() {
                let inc = abs_dep.join("include").to_string_lossy().to_string();
                cflags.push(format!("-I{inc}"));
                include_paths.push(inc);
            }
            if abs_dep.join("lib").exists() {
                let lib = abs_dep.join("lib").to_string_lossy().to_string();
                ldflags.push(format!("-L{lib}"));
                library_paths.push(lib);
            }
            if abs_dep.join("lib/pkgconfig").exists() {
                pkg_config_paths.push(
                    abs_dep.join("lib/pkgconfig").to_string_lossy().to_string(),
                );
            }
        }

        // -- 3. Prepare install destination --------------------------------
        let install_abs = sysroot_path.join("usr/local");
        std::fs::create_dir_all(&install_abs)?;

        // -- 4. Ensure log directory exists --------------------------------
        if let Some(parent) = log_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // -- 5. Construct hermetic PATH ------------------------------------
        // Only system toolchain directories. Xcode CLT is added when present
        // so that `xcrun`, `dsymutil`, and similar Apple tools are available.
        let mut path_dirs = vec![
            "/usr/bin".to_string(),
            "/bin".to_string(),
            "/usr/sbin".to_string(),
            "/sbin".to_string(),
        ];
        let xcode_clt = Path::new("/Library/Developer/CommandLineTools/usr/bin");
        if xcode_clt.is_dir() {
            path_dirs.insert(0, xcode_clt.to_string_lossy().to_string());
        }

        // -- 6. Build the command ------------------------------------------
        let mut cmd = Command::new("/bin/sh");

        // Start from a blank slate so host env vars never leak in.
        cmd.env_clear();

        cmd.arg("-c")
            .arg(script)
            .current_dir(sysroot_path.join("src"))
            // Minimal system
            .env("PATH", path_dirs.join(":"))
            .env("HOME", &sysroot_path)
            .env("TERM", "dumb")
            .env("LANG", "en_US.UTF-8")
            // Toolchain
            .env("CC", "clang")
            .env("CXX", "clang++")
            .env("ARCH", std::env::consts::ARCH)
            // Install paths
            .env("PREFIX", &install_abs)
            .env("OUTPUT", &install_abs)
            .env("DESTDIR", "")
            .env("JOBS", num_cpus::get().to_string())
            // Dependencies root
            .env("DEPS_DIR", &deps_dir)
            // Reproducibility
            .env("MACOSX_DEPLOYMENT_TARGET", MACOSX_DEPLOYMENT_TARGET)
            .env("SOURCE_DATE_EPOCH", SOURCE_DATE_EPOCH);

        // Compiler / linker flags aggregated from all dependencies
        if !cflags.is_empty() {
            let flags = cflags.join(" ");
            cmd.env("CFLAGS", &flags);
            cmd.env("CPPFLAGS", &flags);
        }
        if !ldflags.is_empty() {
            cmd.env("LDFLAGS", ldflags.join(" "));
        }

        // Colon-separated search paths (used by pkg-config, clang, dyld)
        if !include_paths.is_empty() {
            let joined = include_paths.join(":");
            cmd.env("CPATH", &joined);
            cmd.env("C_INCLUDE_PATH", &joined);
            cmd.env("CPLUS_INCLUDE_PATH", &joined);
        }
        if !library_paths.is_empty() {
            let joined = library_paths.join(":");
            cmd.env("LIBRARY_PATH", &joined);
            cmd.env("DYLD_LIBRARY_PATH", &joined);
        }
        if !pkg_config_paths.is_empty() {
            cmd.env("PKG_CONFIG_PATH", pkg_config_paths.join(":"));
        }

        // Per-dependency env vars (DEP_OPENSSL=<sysroot>/deps/openssl, etc.)
        for (key, value) in &per_dep_env {
            cmd.env(key, value);
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

        // -- 7. Extract output ---------------------------------------------
        if output_path.exists() {
            std::fs::remove_dir_all(output_path)?;
        }

        // Prefer rename (atomic, instant on same filesystem) with copy fallback.
        if std::fs::rename(&install_abs, output_path).is_err() {
            copy_dir_all(&install_abs, output_path)?;
        }

        Ok(())
    }
}

/// Recursively copy a directory tree from `src` to `dst`.
///
/// Uses `fs_extra` for robust recursive copying with overwrite semantics.
///
/// # Errors
///
/// Returns an error if any file or directory cannot be copied.
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

/// Read the last N lines from a file efficiently.
///
/// Instead of loading the entire file, we seek to near the end and read a fixed-size
/// tail buffer. This prevents OOM on large build logs (e.g., compiling LLVM).
fn read_last_lines(path: &Path, n: usize) -> Result<String> {
    use std::fs::File;
    use std::io::{Read, Seek, SeekFrom};

    // Read at most 16KB from the end (enough for ~400 lines at 40 chars each)
    const TAIL_SIZE: u64 = 16 * 1024;

    let mut file = File::open(path)?;
    let file_len = file.metadata()?.len();

    let seek_pos = file_len.saturating_sub(TAIL_SIZE);
    file.seek(SeekFrom::Start(seek_pos))?;

    let mut buffer = String::new();
    file.read_to_string(&mut buffer)?;

    // If we seeked mid-file, skip the first (partial) line in-place
    let content = if seek_pos > 0 {
        buffer
            .find('\n')
            .map_or(buffer.as_str(), |idx| &buffer[idx + 1..])
    } else {
        &buffer
    };

    // Take only the last N lines
    let lines: Vec<&str> = content.lines().collect();
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
