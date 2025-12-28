//! Download module with streaming BLAKE3 verification
//!
//! Downloads files while simultaneously hashing them for integrity verification.

use std::io::Write;
use std::path::Path;

use blake3::Hasher;
use futures::StreamExt;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use reqwest::Client;
use thiserror::Error;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

#[derive(Error, Debug)]
pub enum DownloadError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Hash mismatch: expected {expected}, got {actual}")]
    HashMismatch { expected: String, actual: String },
}

/// Create a multi-progress container for parallel downloads
pub fn create_multi_progress() -> MultiProgress {
    MultiProgress::new()
}

/// Download a file with progress bar, streaming to disk
pub async fn download_with_progress(
    client: &Client,
    url: &str,
    dest: &Path,
) -> Result<u64, DownloadError> {
    let response = client.get(url).send().await?.error_for_status()?;
    let total_size = response.content_length().unwrap_or(0);

    let pb = create_progress_bar(total_size);
    pb.set_message(format!("Downloading {}", url.split('/').last().unwrap_or("file")));

    let mut file = File::create(dest).await?;
    let mut stream = response.bytes_stream();
    let mut downloaded: u64 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;
        pb.set_position(downloaded);
    }

    pb.finish_with_message("Download complete");
    Ok(downloaded)
}

/// Download a file while verifying its BLAKE3 hash
///
/// The hash is computed while streaming, so verification is instant on completion.
pub async fn download_and_verify(
    client: &Client,
    url: &str,
    dest: &Path,
    expected_hash: &str,
) -> Result<String, DownloadError> {
    let response = client.get(url).send().await?.error_for_status()?;
    let total_size = response.content_length().unwrap_or(0);

    let pb = create_progress_bar(total_size);
    let filename = url.split('/').last().unwrap_or("file");
    pb.set_message(filename.to_string());

    let mut file = File::create(dest).await?;
    let mut stream = response.bytes_stream();
    let mut hasher = Hasher::new();
    let mut downloaded: u64 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        
        // Write to file and hash simultaneously
        file.write_all(&chunk).await?;
        hasher.write_all(&chunk)?;
        
        downloaded += chunk.len() as u64;
        pb.set_position(downloaded);
    }

    file.flush().await?;
    let actual_hash = hasher.finalize().to_hex().to_string();

    if actual_hash != expected_hash {
        pb.finish_with_message(format!("✗ {filename}"));
        // Clean up failed download
        tokio::fs::remove_file(dest).await.ok();
        return Err(DownloadError::HashMismatch {
            expected: expected_hash.to_string(),
            actual: actual_hash,
        });
    }

    pb.finish_with_message(format!("✓ {filename}"));
    Ok(actual_hash)
}

/// Create a beautifully styled progress bar (uv-inspired)
fn create_progress_bar(total: u64) -> ProgressBar {
    let pb = ProgressBar::new(total);
    
    let style = ProgressStyle::default_bar()
        .template("  {spinner:.green} {msg:32!.cyan} [{bar:40.green/dim}] {bytes:>10}/{total_bytes:<10} ({eta})")
        .unwrap()
        .progress_chars("━━╺")
        .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]);
    
    pb.set_style(style);
    pb.enable_steady_tick(std::time::Duration::from_millis(80));
    pb
}

/// Create a spinner for non-progress operations
pub fn create_spinner(msg: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    
    let style = ProgressStyle::default_spinner()
        .template("  {spinner:.cyan} {msg}")
        .unwrap()
        .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", "✓"]);
    
    pb.set_style(style);
    pb.set_message(msg.to_string());
    pb.enable_steady_tick(std::time::Duration::from_millis(80));
    pb
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_download_and_verify() {
        // Use a small, stable file for testing
        let url = "https://httpbin.org/bytes/1024";
        let client = Client::new();
        let dir = tempdir().unwrap();
        let dest = dir.path().join("test.bin");

        // Just test that download works (can't verify hash of random bytes)
        let result = download_with_progress(&client, url, &dest).await;
        assert!(result.is_ok());
        assert!(dest.exists());
    }
}
