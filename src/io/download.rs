//! Download module with clean inline progress
//!
//! Uses MultiProgress for parallel downloads that don't overlap.

use std::io::Write;
use std::path::Path;
use std::sync::Arc;

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

/// Shared multi-progress for coordinated parallel downloads
pub fn create_multi_progress() -> Arc<MultiProgress> {
    Arc::new(MultiProgress::new())
}

/// Download with streaming BLAKE3 verification (for use with managed ProgressBar)
pub async fn download_and_verify_mp(
    client: &Client,
    url: &str,
    dest: &Path,
    expected_hash: &str,
    pb: &ProgressBar,
) -> Result<String, DownloadError> {
    let response = client.get(url).send().await?.error_for_status()?;
    let total_size = response.content_length().unwrap_or(0);
    pb.set_length(total_size);
    let filename = extract_filename(url);
    pb.set_message(filename.clone());

    let mut file = File::create(dest).await?;
    let mut stream = response.bytes_stream();
    let mut hasher = Hasher::new();
    let mut downloaded: u64 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        hasher.write_all(&chunk)?;
        downloaded += chunk.len() as u64;
        pb.set_position(downloaded);
    }

    file.flush().await?;
    let actual_hash = hasher.finalize().to_hex().to_string();

    if actual_hash != expected_hash {
        let fail_msg = format!("✗ {:<30}", filename);
        pb.set_message(fail_msg);
        pb.finish_and_clear();
        tokio::fs::remove_file(dest).await.ok();
        return Err(DownloadError::HashMismatch {
            expected: expected_hash.to_string(),
            actual: actual_hash,
        });
    }

    // Update message in-place with success, then finish
    let size_str = format_size(total_size);
    let done_msg = format!("✔ {:<24} {}", filename, size_str);
    pb.set_message(done_msg);
    pb.finish();
    Ok(actual_hash)
}

/// Download with streaming BLAKE3 verification (standalone)
pub async fn download_and_verify(
    client: &Client,
    url: &str,
    dest: &Path,
    expected_hash: &str,
) -> Result<String, DownloadError> {
    let mp = MultiProgress::new();
    let pb = mp.add(create_inline_progress(0));
    download_and_verify_mp(client, url, dest, expected_hash, &pb).await
}

/// Download a file with progress (standalone)
pub async fn download_with_progress(
    client: &Client,
    url: &str,
    dest: &Path,
) -> Result<u64, DownloadError> {
    let response = client.get(url).send().await?.error_for_status()?;
    let total_size = response.content_length().unwrap_or(0);

    let pb = create_inline_progress(total_size);
    let filename = extract_filename(url);
    pb.set_message(filename.clone());

    let mut file = File::create(dest).await?;
    let mut stream = response.bytes_stream();
    let mut downloaded: u64 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        downloaded += chunk.len() as u64;
        pb.set_position(downloaded);
    }

    let done_msg = format!("✔ {:<24}", filename);
    pb.set_message(done_msg);
    pb.finish();
    Ok(downloaded)
}

/// Extract clean filename (max 24 chars for alignment)
fn extract_filename(url: &str) -> String {
    let name = url.split('/').last().unwrap_or("file");
    if name.len() > 24 {
        format!("{}...", &name[..21])
    } else {
        name.to_string()
    }
}

/// Format bytes as human-readable
fn format_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("[{} B]", bytes)
    } else if bytes < 1024 * 1024 {
        format!("[{:.1} KiB]", bytes as f64 / 1024.0)
    } else {
        format!("[{:.1} MiB]", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Clean inline progress bar - stays on one line
fn create_inline_progress(total: u64) -> ProgressBar {
    let pb = ProgressBar::new(total);
    
    // Single consistent style throughout - message includes status
    let style = ProgressStyle::default_bar()
        .template("  {msg} {bar:20.dim} {percent:>3}%")
        .unwrap()
        .progress_chars("━╸ ");
    
    pb.set_style(style);
    pb.enable_steady_tick(std::time::Duration::from_millis(100));
    pb
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_download_and_verify() {
        let url = "https://httpbin.org/bytes/1024";
        let client = Client::new();
        let dir = tempdir().unwrap();
        let dest = dir.path().join("test.bin");

        let result = download_with_progress(&client, url, &dest).await;
        assert!(result.is_ok());
        assert!(dest.exists());
    }
}
