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
        output.fail(pkg_name, &format_size(total_size), "hash mismatch");
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
/// Download, Cache, and Pipelined Extract (Async)
///
/// 1. Stream from Network
/// 2. Tee:
///    - Path A: Write to Cache File + Hash (Main loop)
///    - Path B: Send to Channel -> StreamReader -> Zstd -> Tar Unpack (Spawned Task)
/// 3. Verify Hash
pub async fn download_and_extract(
    client: &Client,
    pkg_name: &str,
    url: &str,
    cache_dest: &Path,
    extract_dest: &Path,
    expected_hash: &str,
    output: &CliOutput,
) -> Result<String, DownloadError> {
    use async_compression::tokio::bufread::ZstdDecoder;
    use tokio_tar::Archive;
    use tokio_util::io::StreamReader;

    let user_agent = format!("apl/{}", env!("CARGO_PKG_VERSION"));

    let head_resp = client
        .head(url)
        .header(reqwest::header::USER_AGENT, &user_agent)
        .send()
        .await?;

    let total_size = head_resp.content_length().unwrap_or(0);
    // Note: We ignore accept_ranges for pipelined extraction, we prefer sequential stream

    output.set_downloading(pkg_name, "", total_size);

    let response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, &user_agent)
        .send()
        .await?
        .error_for_status()?;

    let mut stream = response.bytes_stream();
    let mut file = File::create(cache_dest).await?;
    let mut hasher = Hasher::new();
    let mut downloaded: u64 = 0;

    // Channel for Pipeline (Download -> Extract)
    // 64KB buffer
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<bytes::Bytes, std::io::Error>>(32);

    // Convert Receiver to AsyncRead
    let stream_reader = StreamReader::new(tokio_stream::wrappers::ReceiverStream::new(rx));

    // Detect compression format from URL
    let is_gzip = url.ends_with(".tar.gz") || url.ends_with(".tgz");
    let is_zip = url.ends_with(".zip");

    // For zip files, we can't pipeline - download first, then extract
    if is_zip {
        drop(tx); // Don't use pipelined extraction for zip

        // Standard download loop
        while let Some(chunk_res) = stream.next().await {
            let chunk = chunk_res?;
            file.write_all(&chunk).await?;
            hasher.write_all(&chunk)?;
            downloaded += chunk.len() as u64;
            output.update_download(pkg_name, downloaded);
        }
        file.flush().await?;

        let actual_hash = hasher.finalize().to_hex().to_string();
        if actual_hash != expected_hash {
            output.fail(pkg_name, &format_size(downloaded), "hash mismatch");
            tokio::fs::remove_file(cache_dest).await.ok();
            return Err(DownloadError::HashMismatch {
                expected: expected_hash.to_string(),
                actual: actual_hash,
            });
        }

        // Sync zip extraction
        let cache_path = cache_dest.to_path_buf();
        let extract_path = extract_dest.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let file = std::fs::File::open(&cache_path)?;
            let mut archive = zip::ZipArchive::new(file)?;
            archive.extract(&extract_path)?;
            Ok::<(), std::io::Error>(())
        })
        .await
        .map_err(std::io::Error::other)??;

        return Ok(actual_hash);
    }

    // Spawn Extractor Task (for tar archives)
    let extract_dest_owned = extract_dest.to_path_buf();
    let extractor_handle = tokio::spawn(async move {
        if is_gzip {
            use async_compression::tokio::bufread::GzipDecoder;
            let decoder = GzipDecoder::new(stream_reader);
            let mut archive = Archive::new(decoder);
            archive.unpack(&extract_dest_owned).await?;
        } else {
            let decoder = ZstdDecoder::new(stream_reader);
            let mut archive = Archive::new(decoder);
            archive.unpack(&extract_dest_owned).await?;
        }
        Ok::<(), std::io::Error>(())
    });

    // Main Loop: Download -> Cache + Hash + Pipe
    while let Some(chunk_res) = stream.next().await {
        let chunk = chunk_res?;
        // Write to Cache
        file.write_all(&chunk).await?;
        // Update Hash
        hasher.write_all(&chunk)?;

        downloaded += chunk.len() as u64;
        output.update_download(pkg_name, downloaded);

        // Pipe to Extractor (Clone bytes is cheap, it's Arc)
        if tx.send(Ok(chunk)).await.is_err() {
            // Receiver dropped (Extractor failed?), stop downloading
            return Err(
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, "Extractor died").into(),
            );
        }
    }
    drop(tx); // Close channel

    file.flush().await?;
    let actual_hash = hasher.finalize().to_hex().to_string();

    // Verify Hash FIRST
    if actual_hash != expected_hash {
        output.fail(pkg_name, &format_size(downloaded), "hash mismatch");
        tokio::fs::remove_file(cache_dest).await.ok();
        // Also clean up partial extraction? Install caller handles temp dir cleanup.
        return Err(DownloadError::HashMismatch {
            expected: expected_hash.to_string(),
            actual: actual_hash,
        });
    }

    // Wait for extraction to finish
    match extractor_handle.await {
        Ok(Ok(_)) => Ok(actual_hash),
        Ok(Err(e)) => Err(DownloadError::Io(e)),
        Err(e) => Err(DownloadError::Io(std::io::Error::other(
            e,
        ))),
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
