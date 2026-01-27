//! Sysroot Builder using APFS clonefile (CoW)
//!
//! Verified support: macOS 10.12+ (APFS)

use anyhow::Result;
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

// FFI for macOS clonefile(2) syscall. This is the only foreign function we
// bind directly; everything else goes through safe Rust crates.
#[allow(unsafe_code)]
unsafe extern "C" {
    // flags: 0 or CLONE_NOFOLLOW (1) | CLONE_NOOWNERCOPY (2)
    fn clonefile(src: *const libc::c_char, dst: *const libc::c_char, flags: u32) -> libc::c_int;
}

const CLONE_NOFOLLOW: u32 = 0x0001;

/// A hermetic build environment using Copy-On-Write logic
#[derive(Debug)]
pub struct Sysroot {
    temp_dir: tempfile::TempDir,
}

impl Sysroot {
    /// Create a new disposable sysroot in a temp directory.
    ///
    /// The directory is created under the APL temp path so that it resides
    /// on the same APFS volume as the store, enabling instant
    /// `clonefile(2)` operations.
    ///
    /// # Errors
    ///
    /// Returns an error if the temp directory cannot be created.
    pub fn new() -> Result<Self> {
        let tmp = crate::tmp_path();
        std::fs::create_dir_all(&tmp)?;

        let temp_dir = tempfile::Builder::new()
            .prefix("apl-build-")
            .tempdir_in(&tmp)?;

        Ok(Self { temp_dir })
    }

    /// Access the root path
    pub fn path(&self) -> &Path {
        self.temp_dir.path()
    }

    /// Mount (CoW Clone) a single package/directory into the sysroot
    ///
    /// `source`: Path to the package in the Store (e.g. ~/.apl/store/openssl-1.1)
    /// `target_rel`: Where to put it relative to sysroot (e.g. "usr/local")
    #[allow(unsafe_code)]
    pub fn mount(&self, source: &Path, target_rel: &Path) -> Result<()> {
        let dest = self.temp_dir.path().join(target_rel);

        // Ensure parent exists
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // Check source existence
        if !source.exists() {
            anyhow::bail!("Source does not exist: {}", source.display());
        }

        let c_src = CString::new(source.as_os_str().as_bytes())?;
        let c_dst = CString::new(dest.as_os_str().as_bytes())?;

        // SAFETY: Both CStrings are valid null-terminated paths derived from
        // verified Path values. clonefile(2) is the macOS syscall for APFS
        // copy-on-write cloning; it only reads the path pointers.
        let ret = unsafe { clonefile(c_src.as_ptr(), c_dst.as_ptr(), CLONE_NOFOLLOW) };

        if ret != 0 {
            let err = std::io::Error::last_os_error();
            anyhow::bail!(
                "clonefile failed: {err} (src={}, dest={})",
                source.display(),
                dest.display()
            );
        }

        Ok(())
    }
}
