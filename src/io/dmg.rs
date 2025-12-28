//! DMG handling via hdiutil

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use anyhow::{Result, Context, bail};

/// Represents a mounted DMG. Dropping this struct will detach the volume.
pub struct MountPoint {
    pub path: PathBuf,
}

impl Drop for MountPoint {
    fn drop(&mut self) {
        let _ = detach(&self.path);
    }
}

/// Attach a DMG file and return its mount point
pub fn attach(dmg_path: &Path) -> Result<MountPoint> {
    let output = Command::new("hdiutil")
        .arg("attach")
        .arg("-nobrowse")
        .arg("-readonly")
        .arg(dmg_path)
        .output()
        .context("Failed to execute hdiutil")?;
        
    if !output.status.success() {
        bail!("hdiutil attach failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    
    let stdout = String::from_utf8_lossy(&output.stdout);
    
    // Parse output for /Volumes/...
    // Format: /dev/diskXsY <TYPE> <MOUNTPOINT>
    // Separator is usually tabs/spaces.
    
    for line in stdout.lines() {
        if let Some(idx) = line.find("/Volumes/") {
            let path = line[idx..].trim();
            return Ok(MountPoint { path: PathBuf::from(path) });
        }
    }
    
    bail!("Could not find mount point in hdiutil output");
}

/// Detach a volume
pub fn detach(mount_point: &Path) -> Result<()> {
    // Retry logic often good for detach (busy resource)
    for _ in 0..3 {
        let status = Command::new("hdiutil")
            .arg("detach")
            .arg(mount_point)
            .arg("-force")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
            
        if let Ok(s) = status {
            if s.success() {
                return Ok(());
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    
    bail!("Failed to detach {}", mount_point.display());
}
