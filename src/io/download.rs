//! Download module with simple progress output
//!
//! Uses manual cursor control for reliable progress display.

use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use blake3::Hasher;
use futures::StreamExt;
use reqwest::Client;
use thiserror::Error;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

use crate::io::output::CliOutput;

#[derive(Error, Debug)]
pub enum DownloadError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Hash mismatch: expected {expected}, got {actual}")]
    HashMismatch { expected: String, actual: String },
}

/// Download with streaming BLAKE3 verification (for use with CliOutput)
pub async fn download_and_verify_mp(
    client: &Client,
    pkg_name: &str,
    url: &str,
    dest: &Path,
    expected_hash: &str,
    output: &CliOutput,
) -> Result<String, DownloadError> {
    let user_agent = format!("apl/{}", env!("CARGO_PKG_VERSION"));

    let head_resp = client
        .head(url)
        .header(reqwest::header::USER_AGENT, &user_agent)
        .send()
        .await?;

    let total_size = head_resp.content_length().unwrap_or(0);
    let accept_ranges = head_resp
        .headers()
        .get(reqwest::header::ACCEPT_RANGES)
        .map(|v| v == "bytes")
        .unwrap_or(false);

    // Set the total bytes for progress display
    output.set_downloading(pkg_name, "", total_size);

    if total_size > 10 * 1024 * 1024 && accept_ranges {
        return download_chunked(
            client,
            pkg_name,
            url,
            dest,
            expected_hash,
            total_size,
            output,
            &user_agent,
        )
        .await;
    }

    let response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, &user_agent)
        .send()
        .await?
        .error_for_status()?;

    let mut file = File::create(dest).await?;
    let mut stream = response.bytes_stream();
    let mut hasher = Hasher::new();
    let mut downloaded: u64 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        hasher.write_all(&chunk)?;
        downloaded += chunk.len() as u64;
        output.update_download(pkg_name, downloaded);
    }

    file.flush().await?;
    let actual_hash = hasher.finalize().to_hex().to_string();

    if actual_hash != expected_hash {
        output.fail(pkg_name, &format_size(downloaded), "hash mismatch");
        tokio::fs::remove_file(dest).await.ok();
        return Err(DownloadError::HashMismatch {
            expected: expected_hash.to_string(),
            actual: actual_hash,
        });
    }

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

    let head_resp = client
        .head(url)
        .header(reqwest::header::USER_AGENT, &user_agent)
        .send()
        .await?;

    let total_size = head_resp.content_length().unwrap_or(0);
    let accept_ranges = head_resp
        .headers()
        .get(reqwest::header::ACCEPT_RANGES)
        .map(|v| v == "bytes")
        .unwrap_or(false);

    if total_size > 10 * 1024 * 1024 && accept_ranges {
        return download_chunked_simple(client, url, dest, expected_hash, total_size, &user_agent)
            .await;
    }

    let response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, &user_agent)
        .send()
        .await?
        .error_for_status()?;

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

/// Download a file in chunks parallelly
async fn download_chunked(
    client: &Client,
    pkg_name: &str,
    url: &str,
    dest: &Path,
    expected_hash: &str,
    total_size: u64,
    output: &CliOutput,
    user_agent: &str,
) -> Result<String, DownloadError> {
    let chunk_count = if total_size > 50 * 1024 * 1024 { 16 } else { 8 };
    let chunk_size = total_size.div_ceil(chunk_count);
    let mut handles = Vec::new();

    {
        let file = std::fs::File::create(dest)?;
        file.set_len(total_size)?;
    }

    let downloaded = Arc::new(tokio::sync::Mutex::new(0u64));

    for i in 0..chunk_count {
        let start = i * chunk_size;
        let end = std::cmp::min(start + chunk_size - 1, total_size - 1);

        let client = client.clone();
        let url = url.to_string();
        let dest = dest.to_path_buf();
        let output = output.clone();
        let downloaded = downloaded.clone();
        let user_agent_owned = user_agent.to_string();
        let pkg_name = pkg_name.to_string();

        handles.push(tokio::spawn(async move {
            let resp = client
                .get(&url)
                .header(reqwest::header::USER_AGENT, &user_agent_owned)
                .header(reqwest::header::RANGE, format!("bytes={start}-{end}"))
                .send()
                .await?
                .error_for_status()?;

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
                let len = chunk.len() as u64;
                tx.send(chunk)
                    .await
                    .map_err(|_| std::io::Error::other("Channel closed"))?;

                let mut d = downloaded.lock().await;
                *d += len;
                output.update_download(&pkg_name, *d);
            }
            drop(tx);

            write_handle.await.map_err(std::io::Error::other)??;
            Ok::<(), DownloadError>(())
        }));
    }

    for handle in handles {
        handle.await.map_err(std::io::Error::other)??;
    }

    let mut hasher = Hasher::new();
    let mut file = std::fs::File::open(dest)?;
    std::io::copy(&mut file, &mut hasher)?;
    let actual_hash = hasher.finalize().to_hex().to_string();

    if actual_hash != expected_hash {
        output.fail(&pkg_name, &format_size(total_size), "hash mismatch");
        let _ = tokio::fs::remove_file(dest).await;
        return Err(DownloadError::HashMismatch {
            expected: expected_hash.to_string(),
            actual: actual_hash,
        });
    }

    Ok(actual_hash)
}

/// Download a file in chunks parallelly (no progress bar)
async fn download_chunked_simple(
    client: &Client,
    url: &str,
    dest: &Path,
    expected_hash: &str,
    total_size: u64,
    user_agent: &str,
) -> Result<String, DownloadError> {
    let chunk_count = if total_size > 50 * 1024 * 1024 { 16 } else { 8 };
    let chunk_size = total_size.div_ceil(chunk_count);
    let mut handles = Vec::new();

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
            let resp = client
                .get(&url)
                .header(reqwest::header::USER_AGENT, &user_agent_owned)
                .header(reqwest::header::RANGE, format!("bytes={start}-{end}"))
                .send()
                .await?
                .error_for_status()?;

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
                tx.send(chunk)
                    .await
                    .map_err(|_| std::io::Error::other("Channel closed"))?;
            }
            drop(tx);

            write_handle.await.map_err(std::io::Error::other)??;
            Ok::<(), DownloadError>(())
        }));
    }

    for handle in handles {
        handle.await.map_err(std::io::Error::other)??;
    }

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

/// Format bytes as human readable
fn format_size(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = KIB * 1024;
    const GIB: u64 = MIB * 1024;

    if bytes >= GIB {
        format!("{:.1} GiB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn test_download_and_verify() {
        // Note: This test requires network access and a known correct hash
        // Skipping for now as it depends on external service
    }
}
