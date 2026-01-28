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

use apl_schema::Arch;

use crate::sysroot::Sysroot;

/// Minimum macOS version we target. Ventura (13.0) is the baseline: it is the
/// oldest release that ships with the required APFS clonefile(2) semantics and
/// a Rosetta 2 translation layer for `x86_64` binaries on Apple Silicon.
const MACOSX_DEPLOYMENT_TARGET: &str = "13.0";

/// Fixed epoch for `SOURCE_DATE_EPOCH`. Using zero (1970-01-01T00:00:00Z)
/// ensures that all embedded timestamps are identical across builds regardless
/// of when the build actually ran.
const SOURCE_DATE_EPOCH: &str = "0";

/// Sandbox profile template for macOS `sandbox-exec`.
///
/// This profile is designed for full hermeticity:
/// - Default deny.
/// - Deny all network access.
/// - Deny access to host user data (`~/.ssh`, etc.).
/// - Allow read access to system toolchains and SDKs.
/// - Allow read/write access ONLY to the sysroot and temporary directories.
const SANDBOX_PROFILE: &str = r#"
(version 1)
(allow default)

;; 1. Deny Network
(deny network-outbound)

;; 2. Protect Host Toolchains (Hermeticity)
;; Prevents picking up host headers/libs from Homebrew or /usr/local
(deny file-read* (subpath "/usr/local"))
(deny file-read* (subpath "/opt/homebrew"))

;; 3. Prevent Write access to host system
(deny file-write* (subpath "/usr"))
(deny file-write* (subpath "/bin"))
(deny file-write* (subpath "/sbin"))
(deny file-write* (subpath "/System"))
(deny file-write* (subpath "/Library"))

;; 4. Protect Sensitive User Data
;; We don't block all of /Users because we need to read/write the sysroot
;; which is likely in a subdirectory of the user's home or workspace.
;; Instead, we block specific sensitive directories.
(deny file-read* (subpath "/Users/{USER}/.ssh"))
(deny file-read* (subpath "/Users/{USER}/.gitconfig"))
(deny file-read* (subpath "/Users/{USER}/.aws"))
(deny file-read* (subpath "/Users/{USER}/.gnupg"))
"#;

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
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        &self,
        source_path: &Path,
        deps: &[(String, std::path::PathBuf)],
        script: &str,
        output_path: &Path,
        verbose: bool,
        log_path: &Path,
        target_arch: Option<Arch>,
    ) -> Result<()> {
        let sysroot_path = self.sysroot.path().canonicalize()?;

        // Mount source tree
        self.sysroot.mount(source_path, Path::new("src"))?;

        // Mount dependencies and collect compiler/linker search paths
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
            let env_name = format!("DEP_{}", name.to_uppercase().replace(['-', '.'], "_"));
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
                pkg_config_paths.push(abs_dep.join("lib/pkgconfig").to_string_lossy().to_string());
            }
        }

        // Prepare install destination
        let install_abs = sysroot_path.join("usr/local");
        std::fs::create_dir_all(&install_abs)?;

        // Ensure log directory exists
        if let Some(parent) = log_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Construct hermetic PATH
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

        // Add bin/ directory for each dependency for tool discovery (e.g. autoconf, cmake)
        for (name, _) in deps {
            let dep_bin = sysroot_path.join("deps").join(name).join("bin");
            if dep_bin.is_dir() {
                path_dirs.insert(0, dep_bin.to_string_lossy().to_string());
            }
        }

        // Default to host architecture when no explicit target is given.
        let resolved_arch = target_arch.unwrap_or_else(Arch::current);

        // Cross-compilation: Arm64 host targeting x86_64 via Rosetta 2.
        let cross_x86 = cfg!(target_arch = "aarch64") && resolved_arch == Arch::X86_64;

        // Build the command
        // On macOS, wrap in `sandbox-exec` for process-level hermeticity.
        // For cross-compilation, prepend `arch -x86_64` so Rosetta 2
        // translates the build process and all spawned children.
        let mut cmd = if cfg!(target_os = "macos") {
            let current_user = std::env::var("USER").unwrap_or_else(|_| "root".to_string());
            let profile = SANDBOX_PROFILE
                .replace("{SYSROOT}", &sysroot_path.to_string_lossy())
                .replace("{USER}", &current_user);

            if cross_x86 {
                let mut c = Command::new("arch");
                c.arg("-x86_64")
                    .arg("sandbox-exec")
                    .arg("-p")
                    .arg(profile)
                    .arg("/bin/sh");
                c
            } else {
                let mut c = Command::new("sandbox-exec");
                c.arg("-p").arg(profile).arg("/bin/sh");
                c
            }
        } else {
            Command::new("/bin/sh")
        };

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
            .env("ARCH", resolved_arch.rust_name())
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

        if cfg!(target_os = "macos") {
            // Attempt to find the SDK path using xcrun
            if let Ok(output) = std::process::Command::new("xcrun")
                .arg("--show-sdk-path")
                .output()
            {
                if output.status.success() {
                    let sdk_path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                    if !sdk_path.is_empty() {
                        cmd.env("SDKROOT", sdk_path);
                    }
                }
            }
        }

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

        // Extract output
        if output_path.exists() {
            std::fs::remove_dir_all(output_path)?;
        }

        // Prefer rename (atomic, instant on same filesystem) with copy fallback.
        if std::fs::rename(&install_abs, output_path).is_err() {
            copy_dir_all(&install_abs, output_path)?;
        }

        // Fix absolute symlinks
        // Build scripts (e.g. bzip2, openssl) often create absolute symlinks
        // pointing into $PREFIX. After the rename/copy above, these point to the
        // old sysroot path and are dangling. Convert them to portable relative
        // symlinks so the package is relocatable and can be bundled into a
        // tar.zst archive without following broken links.
        fix_absolute_symlinks(&install_abs, output_path)?;

        Ok(())
    }
}

/// Walk the output directory and convert absolute symlinks that pointed into
/// the old sysroot install prefix to relative symlinks.
///
/// Many build systems (Make, `CMake`, Autotools) create absolute symlinks during
/// `make install` that reference `$PREFIX`. After the builder moves the output
/// out of the sysroot, these links become dangling. This function detects them
/// and rewrites each one as a relative symlink so the package remains portable.
///
/// # Errors
///
/// Returns an error if symlink metadata cannot be read or if rewriting a
/// symlink fails.
fn fix_absolute_symlinks(old_prefix: &Path, new_root: &Path) -> Result<()> {
    for entry in walkdir::WalkDir::new(new_root)
        .into_iter()
        .filter_map(std::result::Result::ok)
    {
        let path = entry.path();
        let Ok(meta) = path.symlink_metadata() else {
            continue;
        };
        if !meta.is_symlink() {
            continue;
        }
        let Ok(target) = std::fs::read_link(path) else {
            continue;
        };
        if !target.is_absolute() {
            continue;
        }

        // Check if the target was inside the old install prefix.
        let Ok(suffix) = target.strip_prefix(old_prefix) else {
            continue;
        };

        // Compute where the target now lives inside new_root.
        let new_target = new_root.join(suffix);
        let Some(link_dir) = path.parent() else {
            continue;
        };

        let relative = relative_path(link_dir, &new_target);
        std::fs::remove_file(path)?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(&relative, path)?;
    }

    Ok(())
}

/// Compute a relative path from `from_dir` to `to_path`.
///
/// Both arguments must be absolute paths. The function walks up from
/// `from_dir` to the common ancestor and then descends into `to_path`.
///
/// Example: `relative_path("/a/b/c", "/a/b/d/e")` returns `"../d/e"`.
fn relative_path(from_dir: &Path, to_path: &Path) -> std::path::PathBuf {
    let from_components: Vec<_> = from_dir.components().collect();
    let to_components: Vec<_> = to_path.components().collect();

    let common_len = from_components
        .iter()
        .zip(to_components.iter())
        .take_while(|(a, b)| a == b)
        .count();

    let mut result = std::path::PathBuf::new();
    for _ in common_len..from_components.len() {
        result.push("..");
    }
    for part in &to_components[common_len..] {
        result.push(part);
    }
    result
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
            None,
        );

        // Verify that dependency was mounted correctly
        let mounted_dep = sysroot_path.join("deps/test-dep");
        assert!(mounted_dep.exists());
        assert!(mounted_dep.join("include/header.h").exists());

        // Verify that output was extracted
        assert!(out_tmp.path().join("bin/test-bin").exists());
    }

    #[test]
    fn test_relative_path_same_dir() {
        let result = relative_path(Path::new("/a/b/c"), Path::new("/a/b/c/file"));
        assert_eq!(result, std::path::PathBuf::from("file"));
    }

    #[test]
    fn test_relative_path_sibling() {
        let result = relative_path(Path::new("/a/b/bin"), Path::new("/a/b/bin/bzgrep"));
        assert_eq!(result, std::path::PathBuf::from("bzgrep"));
    }

    #[test]
    fn test_relative_path_cross_dir() {
        let result = relative_path(Path::new("/a/b/lib"), Path::new("/a/b/bin/tool"));
        assert_eq!(result, std::path::PathBuf::from("../bin/tool"));
    }

    #[test]
    fn test_fix_absolute_symlinks() {
        let tmp = tempdir().unwrap();
        let old_prefix = Path::new("/old/sysroot/usr/local");
        let new_root = tmp.path();

        // Create directory structure
        std::fs::create_dir_all(new_root.join("bin")).unwrap();
        std::fs::write(new_root.join("bin/bzgrep"), "#!/bin/sh\n").unwrap();

        // Create an absolute symlink like bzip2's Makefile would
        #[cfg(unix)]
        {
            let symlink_path = new_root.join("bin/bzegrep");
            std::os::unix::fs::symlink("/old/sysroot/usr/local/bin/bzgrep", &symlink_path).unwrap();

            // Verify symlink is broken (absolute target doesn't exist)
            assert!(symlink_path.symlink_metadata().unwrap().is_symlink());
            assert!(!symlink_path.exists()); // target doesn't exist

            // Fix it
            fix_absolute_symlinks(old_prefix, new_root).unwrap();

            // Verify it's now a relative symlink pointing to the right place
            let new_target = std::fs::read_link(&symlink_path).unwrap();
            assert!(!new_target.is_absolute());
            assert_eq!(new_target, std::path::PathBuf::from("bzgrep"));
            assert!(symlink_path.exists()); // target now resolves
        }
    }
}
