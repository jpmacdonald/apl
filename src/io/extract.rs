//! Archive extraction module
//!
//! Handles tar.zst, tar.gz, and other archive formats.

use std::fs::{self, File};
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};

use thiserror::Error;
use zip::ZipArchive;
use zstd::stream::Decoder as ZstdDecoder;

#[derive(Error, Debug)]
pub enum ExtractError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("Unsupported archive format: {0}")]
    UnsupportedFormat(String),

    #[error("Archive error: {0}")]
    Archive(String),
}

/// Information about an extracted file
#[derive(Debug, Clone)]
pub struct ExtractedFile {
    /// Path relative to extraction root
    pub relative_path: PathBuf,
    /// Absolute path on disk
    pub absolute_path: PathBuf,
    /// Whether this is an executable
    pub is_executable: bool,
}

/// Extract a tar.zst archive to a destination directory
pub fn extract_tar_zst(
    archive_path: &Path,
    dest_dir: &Path,
) -> Result<Vec<ExtractedFile>, ExtractError> {
    let file = File::open(archive_path)?;
    let reader = BufReader::new(file);
    let zstd_decoder = ZstdDecoder::new(reader)?;

    extract_tar(zstd_decoder, dest_dir)
}

/// Extract a tar.gz archive to a destination directory
pub fn extract_tar_gz(
    archive_path: &Path,
    dest_dir: &Path,
) -> Result<Vec<ExtractedFile>, ExtractError> {
    let file = File::open(archive_path)?;
    let reader = BufReader::new(file);
    let gz_decoder = flate2::read::GzDecoder::new(reader);

    extract_tar(gz_decoder, dest_dir)
}

/// Extract a tar archive from a reader
fn extract_tar<R: Read>(reader: R, dest_dir: &Path) -> Result<Vec<ExtractedFile>, ExtractError> {
    fs::create_dir_all(dest_dir)?;

    let mut archive = tar::Archive::new(reader);
    let mut extracted_files = Vec::new();

    for entry in archive.entries()? {
        let mut entry = entry?;
        let entry_path = entry.path()?;

        // Skip directories
        if entry.header().entry_type().is_dir() {
            continue;
        }

        // Handle archives with a top-level directory (e.g., "neovim-0.10.0/bin/nvim")
        // by optionally stripping the first component
        let relative_path: PathBuf = entry_path.components().collect();

        // Create parent directories
        let absolute_path = dest_dir.join(&relative_path);

        // Sanitize path to prevent Zip Slip
        if !absolute_path.starts_with(dest_dir) {
            return Err(ExtractError::Archive(format!(
                "Invalid path in archive: {}",
                relative_path.display()
            )));
        }

        if let Some(parent) = absolute_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Extract the file
        entry.unpack(&absolute_path)?;

        // Check if executable (Unix mode has execute bit)
        let is_executable = entry
            .header()
            .mode()
            .map(|m| m & 0o111 != 0)
            .unwrap_or(false);

        extracted_files.push(ExtractedFile {
            relative_path,
            absolute_path,
            is_executable,
        });
    }

    Ok(extracted_files)
}

/// Extract a zip archive
pub fn extract_zip(
    archive_path: &Path,
    dest_dir: &Path,
) -> Result<Vec<ExtractedFile>, ExtractError> {
    let file = File::open(archive_path)?;
    let mut archive = ZipArchive::new(file).map_err(|e| ExtractError::Archive(e.to_string()))?;

    fs::create_dir_all(dest_dir)?;
    let mut extracted_files = Vec::new();

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| ExtractError::Archive(e.to_string()))?;
        let relative_path = match file.enclosed_name() {
            Some(path) => path.to_owned(),
            None => continue,
        };

        if file.is_dir() {
            fs::create_dir_all(dest_dir.join(&relative_path))?;
            continue;
        }

        let absolute_path = dest_dir.join(&relative_path);
        if let Some(p) = absolute_path.parent() {
            fs::create_dir_all(p)?;
        }

        let mut outfile = File::create(&absolute_path)?;
        io::copy(&mut file, &mut outfile)?;

        #[cfg(unix)]
        let is_executable = if let Some(mode) = file.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&absolute_path, fs::Permissions::from_mode(mode))?;
            mode & 0o111 != 0
        } else {
            false
        };
        #[cfg(not(unix))]
        let is_executable = false;

        extracted_files.push(ExtractedFile {
            relative_path,
            absolute_path,
            is_executable,
        });
    }

    Ok(extracted_files)
}

/// Detect archive format from file extension
pub fn detect_format(path: &Path) -> ArchiveFormat {
    let path_str = path.to_string_lossy().to_lowercase();

    if path_str.ends_with(".tar.zst") || path_str.ends_with(".tzst") {
        ArchiveFormat::TarZst
    } else if path_str.ends_with(".tar.gz") || path_str.ends_with(".tgz") {
        ArchiveFormat::TarGz
    } else if path_str.ends_with(".tar") {
        ArchiveFormat::Tar
    } else if path_str.ends_with(".zip") {
        ArchiveFormat::Zip
    } else {
        ArchiveFormat::RawBinary
    }
}

/// Supported archive formats
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveFormat {
    TarZst,
    TarGz,
    Tar,
    Zip,
    RawBinary,
}

/// Extract an archive, auto-detecting format
pub fn extract_auto(
    archive_path: &Path,
    dest_dir: &Path,
) -> Result<Vec<ExtractedFile>, ExtractError> {
    match detect_format(archive_path) {
        ArchiveFormat::TarZst => extract_tar_zst(archive_path, dest_dir),
        ArchiveFormat::TarGz => extract_tar_gz(archive_path, dest_dir),
        ArchiveFormat::Tar => {
            let file = File::open(archive_path)?;
            extract_tar(BufReader::new(file), dest_dir)
        }
        ArchiveFormat::Zip => extract_zip(archive_path, dest_dir),
        ArchiveFormat::RawBinary => {
            // For raw binaries, just copy the file
            fs::create_dir_all(dest_dir)?;
            let filename = archive_path
                .file_name()
                .ok_or_else(|| ExtractError::Archive("Invalid filename".to_string()))?;
            let dest_path = dest_dir.join(filename);
            fs::copy(archive_path, &dest_path)?;

            Ok(vec![ExtractedFile {
                relative_path: PathBuf::from(filename),
                absolute_path: dest_path,
                is_executable: true, // Assume raw binaries are executable
            }])
        }
    }
}

/// Detect if a directory has a single top-level directory and strip it by moving contents up.
pub fn strip_components(dir: &Path) -> io::Result<()> {
    let mut entries: Vec<_> = fs::read_dir(dir)?.filter_map(|e| e.ok()).collect();

    // Filter out hidden files (like .DS_Store)
    entries.retain(|e| !e.file_name().to_string_lossy().starts_with('.'));

    // If there is exactly one entry and it's a directory, move its contents up
    if entries.len() == 1 && entries[0].file_type()?.is_dir() {
        let top_level = entries[0].path();
        let sub_entries: Vec<_> = fs::read_dir(&top_level)?.filter_map(|e| e.ok()).collect();

        for entry in sub_entries {
            let target = dir.join(entry.file_name());
            fs::rename(entry.path(), target)?;
        }

        fs::remove_dir(top_level)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_detect_format() {
        assert_eq!(
            detect_format(Path::new("foo.tar.zst")),
            ArchiveFormat::TarZst
        );
        assert_eq!(detect_format(Path::new("foo.tar.gz")), ArchiveFormat::TarGz);
        assert_eq!(detect_format(Path::new("foo.tgz")), ArchiveFormat::TarGz);
        assert_eq!(detect_format(Path::new("foo")), ArchiveFormat::RawBinary);
    }

    #[test]
    fn test_extract_raw_binary() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("mybin");
        fs::write(&src, b"binary content").unwrap();

        let dest = dir.path().join("extracted");
        let files = extract_auto(&src, &dest).unwrap();

        assert_eq!(files.len(), 1);
        assert!(files[0].absolute_path.exists());
    }

    #[test]
    fn test_detect_format_case_insensitive() {
        assert_eq!(
            detect_format(Path::new("FOO.TAR.ZST")),
            ArchiveFormat::TarZst
        );
        assert_eq!(detect_format(Path::new("bar.TAR.GZ")), ArchiveFormat::TarGz);
        assert_eq!(detect_format(Path::new("BAZ.ZIP")), ArchiveFormat::Zip);
    }

    #[test]
    fn test_detect_format_tar() {
        assert_eq!(detect_format(Path::new("archive.tar")), ArchiveFormat::Tar);
    }

    #[test]
    fn test_detect_format_tzst() {
        assert_eq!(
            detect_format(Path::new("archive.tzst")),
            ArchiveFormat::TarZst
        );
    }

    #[test]
    fn test_extracted_file_paths() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("testbin");
        fs::write(&src, b"#!/bin/sh\necho hello").unwrap();

        let dest = dir.path().join("out");
        let files = extract_auto(&src, &dest).unwrap();

        assert_eq!(files[0].relative_path.to_str(), Some("testbin"));
        assert!(files[0].absolute_path.starts_with(dest));
    }

    #[test]
    fn test_strip_components_simple() {
        let dir = tempdir().unwrap();
        let nested = dir.path().join("nested");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("file.txt"), "content").unwrap();

        strip_components(dir.path()).unwrap();

        assert!(dir.path().join("file.txt").exists());
        assert!(!dir.path().join("nested").exists());
    }

    #[test]
    fn test_strip_components_with_hidden_files() {
        let dir = tempdir().unwrap();
        let nested = dir.path().join("nested");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("file.txt"), "content").unwrap();

        // Create a hidden file (simulation of .DS_Store)
        fs::write(dir.path().join(".DS_Store"), "junk").unwrap();

        strip_components(dir.path()).unwrap();

        // Should still strip because .DS_Store is ignored
        assert!(dir.path().join("file.txt").exists());
        assert!(!dir.path().join("nested").exists());
        // .DS_Store should remain (or at least not prevent stripping)
        assert!(dir.path().join(".DS_Store").exists());
    }
}
