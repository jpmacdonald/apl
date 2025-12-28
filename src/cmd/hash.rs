//! Hash command

use anyhow::Result;
use std::path::PathBuf;

/// Compute BLAKE3 hash of files
pub fn hash(files: &[PathBuf]) -> Result<()> {
    for file in files {
        let hash = compute_file_hash(file)?;
        println!("{} {}", hash, file.display());
    }
    Ok(())
}

/// Compute BLAKE3 hash of a file (streaming)
fn compute_file_hash(path: &std::path::Path) -> Result<String> {
    use std::io::Read;
    
    let mut file = std::fs::File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0u8; 65536]; // 64KB buffer
    
    loop {
        let bytes_read = file.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }
    
    Ok(hasher.finalize().to_hex().to_string())
}
