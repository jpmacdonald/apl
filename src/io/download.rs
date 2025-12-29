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
    // Initial check for size and ranges using HEAD
    let user_agent = format!("apl/{}", env!("CARGO_PKG_VERSION"));
    let head_resp = client.head(url)
        .header(reqwest::header::USER_AGENT, &user_agent)
        .send().await?;
    
    let total_size = head_resp.content_length().unwrap_or(0);
    let accept_ranges = head_resp.headers()
        .get(reqwest::header::ACCEPT_RANGES)
        .map(|v| v == "bytes")
        .unwrap_or(false);

    // Update PB length if we have it now (and it wasn't set before)
    if total_size > 0 {
        pb.set_length(total_size);
    }

    // Use chunked download for files > 10MB if ranges are supported
    if total_size > 10 * 1024 * 1024 && accept_ranges {
        return download_chunked(client, url, dest, expected_hash, total_size, pb, &user_agent).await;
    }

    let response = client.get(url)
        .header(reqwest::header::USER_AGENT, &user_agent)
        .send().await?.error_for_status()?;
    
    // If we receive a content-length header here, update PB
    if let Some(len) = response.content_length() {
        if len > 0 {
            pb.set_length(len);
        }
    }
    
    let filename = extract_filename(url);
    // Do NOT overwrite the message (which contains version info) with just filename
    // pb.set_message(filename.clone()); 

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
    // We want the final line to look like:
    //   ✔ name           version    [size]
    // The Output module handles the "✔ name" part via set_style if we call finish_progress_ok.
    // So here we should just return success and let the caller finalize the UI state if needed,
    // OR just leave the PB full.
    // BUT checking install.rs: it calls `output.finish_progress_ok(&pb, &format_size(size));`
    // So we should NOT finish it here if we want `install.rs` to have the final say.
    // However, existing code finished it. Let's see.
    // install.rs:321 calls output.finish_progress_ok.
    // So we should just return successfully and NOT finish the bar here to allow the caller to set the final green check.
    
    Ok(actual_hash)
}

/// Download with streaming BLAKE3 verification (simple, no progress bar)
pub async fn download_and_verify_simple(
    client: &Client,
    url: &str,
    dest: &Path,
    expected_hash: &str,
) -> Result<String, DownloadError> {
    let user_agent = format!("apl/{}", env!("CARGO_PKG_VERSION"));
    
    // Check if server supports range requests for chunked download
    let head_resp = client.head(url)
        .header(reqwest::header::USER_AGENT, &user_agent)
        .send().await?;
    
    let total_size = head_resp.content_length().unwrap_or(0);
    let accept_ranges = head_resp.headers()
        .get(reqwest::header::ACCEPT_RANGES)
        .map(|v| v == "bytes")
        .unwrap_or(false);

    // Use chunked download for large files if supported
    if total_size > 10 * 1024 * 1024 && accept_ranges {
        return download_chunked_simple(client, url, dest, expected_hash, total_size, &user_agent).await;
    }

    // Simple streaming download
    let response = client.get(url)
        .header(reqwest::header::USER_AGENT, &user_agent)
        .send().await?.error_for_status()?;
    
    let mut file = File::create(dest).await?;
    let mut stream = response.bytes_stream();
    let mut hasher = Hasher::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        hasher.write_all(&chunk)?;
    }

    file.flush().await?;
    let actual_hash = hasher.finalize().to_hex().to_string();

    if actual_hash != expected_hash {
        tokio::fs::remove_file(dest).await.ok();
        return Err(DownloadError::HashMismatch {
            expected: expected_hash.to_string(),
            actual: actual_hash,
        });
    }
    
    Ok(actual_hash)
}

/// Download with streaming BLAKE3 verification (standalone, legacy)
pub async fn download_and_verify(
    client: &Client,
    url: &str,
    dest: &Path,
    expected_hash: &str,
) -> Result<String, DownloadError> {
    download_and_verify_simple(client, url, dest, expected_hash).await
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

/// Download a file in chunks parallelly
async fn download_chunked(
    client: &Client,
    url: &str,
    dest: &std::path::Path,
    expected_hash: &str,
    total_size: u64,
    pb: &ProgressBar,
    user_agent: &str,
) -> Result<String, DownloadError> {
    let chunk_count = if total_size > 50 * 1024 * 1024 { 16 } else { 8 };
    let chunk_size = (total_size + chunk_count - 1) / chunk_count;
    let mut handles = Vec::new();
    
    // Create the file and set its size
    {
        let file = std::fs::File::create(dest)?;
        file.set_len(total_size)?;
    }

    let downloaded = Arc::new(tokio::sync::Mutex::new(0u64));
    let filename = extract_filename(url);

    for i in 0..chunk_count {
        let start = i * chunk_size;
        let end = std::cmp::min(start + chunk_size - 1, total_size - 1);
        
        let client = client.clone();
        let url = url.to_string();
        let dest = dest.to_path_buf();
        let pb = pb.clone();
        let downloaded = downloaded.clone();
        let user_agent_owned = user_agent.to_string();

        handles.push(tokio::spawn(async move {
            let resp = client.get(&url)
                .header(reqwest::header::USER_AGENT, &user_agent_owned)
                .header(reqwest::header::RANGE, format!("bytes={}-{}", start, end))
                .send().await?.error_for_status()?;
            
            let mut body = resp.bytes_stream();
            
            // Note: We use spawn_blocking for the entire segment write loop
            // to ensure we don't block the async executor and get full throughput.
            let (tx, mut rx) = tokio::sync::mpsc::channel::<bytes::Bytes>(32);
            
            let write_handle = tokio::task::spawn_blocking(move || {
                use std::io::{Seek, SeekFrom, Write};
                let mut file = std::fs::OpenOptions::new().write(true).open(&dest)?;
                file.seek(SeekFrom::Start(start))?;
                
                while let Some(chunk) = rx.blocking_recv() {
                    file.write_all(&chunk)?;
                }
                file.flush()?;
                Ok::<(), std::io::Error>(())
            });

            while let Some(chunk) = body.next().await {
                let chunk = chunk?;
                let len = chunk.len() as u64;
                tx.send(chunk).await.map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "Channel closed"))?;
                
                let mut d = downloaded.lock().await;
                *d += len;
                pb.set_position(*d);
            }
            drop(tx); // Signal end of stream
            
            write_handle.await.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))??;
            Ok::<(), DownloadError>(())
        }));
    }

    for handle in handles {
        handle.await.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))??;
    }

    // Verify final hash
    let mut hasher = Hasher::new();
    let mut file = std::fs::File::open(dest)?;
    std::io::copy(&mut file, &mut hasher)?;
    let actual_hash = hasher.finalize().to_hex().to_string();

    if actual_hash != expected_hash {
        pb.set_message(format!("✗ {:<30}", filename));
        pb.finish_and_clear();
        let _ = tokio::fs::remove_file(dest).await;
        return Err(DownloadError::HashMismatch {
            expected: expected_hash.to_string(),
            actual: actual_hash,
        });
    }

    // Don't finish the bar here - let the caller handle finalization
    // via output.finish_progress_ok() for consistent styling
    Ok(actual_hash)
}

/// Download a file in chunks parallelly (no progress bar)
async fn download_chunked_simple(
    client: &Client,
    url: &str,
    dest: &std::path::Path,
    expected_hash: &str,
    total_size: u64,
    user_agent: &str,
) -> Result<String, DownloadError> {
    let chunk_count = if total_size > 50 * 1024 * 1024 { 16 } else { 8 };
    let chunk_size = (total_size + chunk_count - 1) / chunk_count;
    let mut handles = Vec::new();
    
    // Create the file and set its size
    {
        let file = std::fs::File::create(dest)?;
        file.set_len(total_size)?;
    }

    for i in 0..chunk_count {
        let start = i * chunk_size;
        let end = std::cmp::min(start + chunk_size - 1, total_size - 1);
        
        let client = client.clone();
        let url = url.to_string();
        let dest = dest.to_path_buf();
        let user_agent_owned = user_agent.to_string();

        handles.push(tokio::spawn(async move {
            let resp = client.get(&url)
                .header(reqwest::header::USER_AGENT, &user_agent_owned)
                .header(reqwest::header::RANGE, format!("bytes={}-{}", start, end))
                .send().await?.error_for_status()?;
            
            let mut body = resp.bytes_stream();
            let (tx, mut rx) = tokio::sync::mpsc::channel::<bytes::Bytes>(32);
            
            let write_handle = tokio::task::spawn_blocking(move || {
                use std::io::{Seek, SeekFrom, Write};
                let mut file = std::fs::OpenOptions::new().write(true).open(&dest)?;
                file.seek(SeekFrom::Start(start))?;
                
                while let Some(chunk) = rx.blocking_recv() {
                    file.write_all(&chunk)?;
                }
                file.flush()?;
                Ok::<(), std::io::Error>(())
            });

            while let Some(chunk) = body.next().await {
                let chunk = chunk?;
                tx.send(chunk).await.map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "Channel closed"))?;
            }
            drop(tx);
            
            write_handle.await.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))??;
            Ok::<(), DownloadError>(())
        }));
    }

    for handle in handles {
        handle.await.map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))??;
    }

    // Verify final hash
    let mut hasher = Hasher::new();
    let mut file = std::fs::File::open(dest)?;
    std::io::copy(&mut file, &mut hasher)?;
    let actual_hash = hasher.finalize().to_hex().to_string();

    if actual_hash != expected_hash {
        let _ = tokio::fs::remove_file(dest).await;
        return Err(DownloadError::HashMismatch {
            expected: expected_hash.to_string(),
            actual: actual_hash,
        });
    }

    Ok(actual_hash)
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
