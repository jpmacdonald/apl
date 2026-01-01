//! DMG handling via hdiutil

use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// Represents a mounted DMG. Dropping this struct will detach the volume.
#[derive(Debug)]
pub struct MountPoint {
    pub path: PathBuf,
}

impl Drop for MountPoint {
    fn drop(&mut self) {
        let _ = detach(&self.path);
    }
}

/// Attach a DMG file and return its mount point
///
/// # Timeout
/// Will timeout after 30 seconds to prevent hanging on interactive DMGs
pub fn attach(dmg_path: &Path) -> Result<MountPoint> {
    if !dmg_path.exists() {
        bail!("DMG file not found: {}", dmg_path.display());
    }

    tracing::debug!("Attaching DMG: {}", dmg_path.display());

    // Spawn with comprehensive flags
    let mut child = Command::new("hdiutil")
        .arg("attach")
        .arg("-nobrowse") // Don't open in Finder
        .arg("-readonly") // Mount read-only
        .arg("-noverify") // Skip image verification (faster)
        .arg("-noautoopen") // Don't auto-open volumes
        .arg("-quiet") // Suppress verbose output
        .arg(dmg_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to spawn hdiutil")?;

    // Wait with timeout
    let timeout = Duration::from_secs(30);
    let result = wait_timeout::ChildExt::wait_timeout(&mut child, timeout)
        .context("Failed to wait for hdiutil")?;

    let output = if let Some(status) = result {
        // Process completed within timeout
        let stdout = {
            use std::io::Read;
            let mut buf = Vec::new();
            child.stdout.take().unwrap().read_to_end(&mut buf)?;
            buf
        };
        let stderr = {
            use std::io::Read;
            let mut buf = Vec::new();
            child.stderr.take().unwrap().read_to_end(&mut buf)?;
            buf
        };

        std::process::Output {
            status,
            stdout,
            stderr,
        }
    } else {
        // Timeout - kill the process
        let _ = child.kill();
        let _ = child.wait();
        bail!(
            "hdiutil attach timed out after 30s. This DMG may require user interaction (EULA acceptance). \
             Try manually opening: open '{}'",
            dmg_path.display()
        );
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "hdiutil attach failed: {}\n\
             This may indicate:\n\
             1. DMG requires user interaction (EULA)\n\
             2. DMG is corrupted or incompatible\n\
             3. Insufficient disk space\n\
             4. Another process is using the file",
            stderr
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    tracing::debug!("hdiutil output: {}", stdout);

    // Parse mount point - look for /Volumes/...
    for line in stdout.lines() {
        if let Some(idx) = line.find("/Volumes/") {
            let mount_str = line[idx..].trim();
            let path = PathBuf::from(mount_str);

            if path.exists() {
                tracing::info!("Mounted DMG at: {}", path.display());
                return Ok(MountPoint { path });
            }
        }
    }

    bail!("Could not find mount point in hdiutil output:\n{}", stdout);
}

/// Detach a volume with retries
pub fn detach(mount_point: &Path) -> Result<()> {
    tracing::debug!("Detaching volume: {}", mount_point.display());

    for attempt in 1..=5 {
        let status = Command::new("hdiutil")
            .arg("detach")
            .arg(mount_point)
            .arg("-force")
            .arg("-quiet")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        match status {
            Ok(s) if s.success() => {
                tracing::info!("Successfully detached {}", mount_point.display());
                return Ok(());
            }
            _ => {
                if attempt < 5 {
                    tracing::warn!("Detach attempt {} failed, retrying...", attempt);
                    std::thread::sleep(Duration::from_millis(500));
                }
            }
        }
    }

    bail!(
        "Failed to detach {} after 5 attempts",
        mount_point.display()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_attach_nonexistent_dmg() {
        let result = attach(Path::new("/tmp/nonexistent.dmg"));
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("not found"));
    }

    #[test]
    fn test_detach_nonexistent_volume() {
        let result = detach(Path::new("/Volumes/NonexistentVolume"));
        assert!(result.is_err());
    }
}
