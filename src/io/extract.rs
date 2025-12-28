//! Archive extraction module
//!
//! Handles tar.zst, tar.gz, and other archive formats.

use std::fs::{self, File};
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};

use thiserror::Error;
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
pub fn extract_tar_zst(archive_path: &Path, dest_dir: &Path) -> Result<Vec<ExtractedFile>, ExtractError> {
    let file = File::open(archive_path)?;
    let reader = BufReader::new(file);
    let zstd_decoder = ZstdDecoder::new(reader)?;
    
    extract_tar(zstd_decoder, dest_dir)
}

/// Extract a tar.gz archive to a destination directory
pub fn extract_tar_gz(archive_path: &Path, dest_dir: &Path) -> Result<Vec<ExtractedFile>, ExtractError> {
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
        let relative_path: PathBuf = entry_path
            .components()
            .skip(0) // Keep full path for now; can strip if needed
            .collect();
        
        let absolute_path = dest_dir.join(&relative_path);
        
        // Create parent directories
        if let Some(parent) = absolute_path.parent() {
            fs::create_dir_all(parent)?;
        }
        
        // Extract the file
        entry.unpack(&absolute_path)?;
        
        // Check if executable (Unix mode has execute bit)
        let is_executable = entry.header().mode().map(|m| m & 0o111 != 0).unwrap_or(false);
        
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
    RawBinary,
}

/// Extract an archive, auto-detecting format
pub fn extract_auto(archive_path: &Path, dest_dir: &Path) -> Result<Vec<ExtractedFile>, ExtractError> {
    match detect_format(archive_path) {
        ArchiveFormat::TarZst => extract_tar_zst(archive_path, dest_dir),
        ArchiveFormat::TarGz => extract_tar_gz(archive_path, dest_dir),
        ArchiveFormat::Tar => {
            let file = File::open(archive_path)?;
            extract_tar(BufReader::new(file), dest_dir)
        }
        ArchiveFormat::RawBinary => {
            // For raw binaries, just copy the file
            fs::create_dir_all(dest_dir)?;
            let filename = archive_path.file_name()
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_detect_format() {
        assert_eq!(detect_format(Path::new("foo.tar.zst")), ArchiveFormat::TarZst);
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
}
