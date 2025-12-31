//! Async download and verification module supporting parallel chunking and progress reporting.
//!
//! Handles file downloads with streaming BLAKE3 verification.

use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use blake3::Hasher;
use futures::StreamExt;
use reqwest::Client;
use thiserror::Error;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

use crate::ui::Reporter;

#[derive(Error, Debug)]
pub enum DownloadError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Hash mismatch: expected {expected}, got {actual}")]
    HashMismatch { expected: String, actual: String },
}

/// Downloads and verifies a file, automatically switching to parallel chunking for large files.
pub async fn download_and_verify_mp<R: Reporter + Clone + 'static>(
    client: &Client,
    pkg_name: &str,
    version: &str,
    url: &str,
    dest: &Path,
    expected_hash: &str,
    reporter: &R,
) -> Result<String, DownloadError> {
    let user_agent = crate::USER_AGENT;

    let head_resp = client
        .head(url)
        .header(reqwest::header::USER_AGENT, user_agent)
        .send()
        .await?;

    let total_size = head_resp.content_length().unwrap_or(0);
    let accept_ranges = head_resp
        .headers()
        .get(reqwest::header::ACCEPT_RANGES)
        .map(|v| v == "bytes")
        .unwrap_or(false);

    // Initialize progress state
    reporter.downloading(pkg_name, version, 0, total_size);

    if total_size > 10 * 1024 * 1024 && accept_ranges {
        return download_chunked(
            client,
            pkg_name,
            version,
            url,
            dest,
            expected_hash,
            total_size,
            Some(reporter.clone()),
            &user_agent,
        )
        .await;
    }

    let response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, user_agent)
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
        reporter.downloading(pkg_name, version, downloaded, total_size);
    }

    file.flush().await?;
    let actual_hash = hasher.finalize().to_hex().to_string();

    if actual_hash != expected_hash {
        reporter.failed(pkg_name, &format_size(downloaded), "hash mismatch");
        tokio::fs::remove_file(dest).await.ok();
        return Err(DownloadError::HashMismatch {
            expected: expected_hash.to_string(),
            actual: actual_hash,
        });
    }

    Ok(actual_hash)
}

/// Perform a simple, sequential download with streaming verification.
pub async fn download_and_verify_simple(
    client: &Client,
    url: &str,
    dest: &Path,
    expected_hash: &str,
) -> Result<String, DownloadError> {
    let user_agent = crate::USER_AGENT;

    let head_resp = client
        .head(url)
        .header(reqwest::header::USER_AGENT, user_agent)
        .send()
        .await?;

    let total_size = head_resp.content_length().unwrap_or(0);
    let accept_ranges = head_resp
        .headers()
        .get(reqwest::header::ACCEPT_RANGES)
        .map(|v| v == "bytes")
        .unwrap_or(false);

    if total_size > 10 * 1024 * 1024 && accept_ranges {
        // We pass None as reporter because this is the 'simple' version
        return download_chunked::<crate::ui::Output>(
            client,
            "",
            "",
            url,
            dest,
            expected_hash,
            total_size,
            None,
            &user_agent,
        )
        .await;
    }

    let response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, user_agent)
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

/// Executes a concurrent, chunked download using HTTP Range headers.
async fn download_chunked<R: Reporter + Clone + 'static>(
    client: &Client,
    pkg_name: &str,
    version: &str,
    url: &str,
    dest: &Path,
    expected_hash: &str,
    total_size: u64,
    reporter: Option<R>,
    user_agent: &str,
) -> Result<String, DownloadError> {
    // Determine parallelism based on file size
    let chunk_count = if total_size > 50 * 1024 * 1024 { 16 } else { 8 };
    let chunk_size = total_size.div_ceil(chunk_count);
    let mut handles = Vec::new();

    // Pre-allocate file space
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
        let reporter = reporter.clone();
        let downloaded = downloaded.clone();
        let user_agent_owned = user_agent.to_string();
        let pkg_name = pkg_name.to_string();
        let version = version.to_string();

        handles.push(tokio::spawn(async move {
            let resp = client
                .get(&url)
                .header(reqwest::header::USER_AGENT, &user_agent_owned)
                .header(reqwest::header::RANGE, format!("bytes={start}-{end}"))
                .send()
                .await?
                .error_for_status()?;

            let mut body = resp.bytes_stream();

            // We use a channel to offload file writing to a blocking task
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

                if let Some(rep) = &reporter {
                    let mut d = downloaded.lock().await;
                    *d += len;
                    rep.downloading(&pkg_name, &version, *d, total_size);
                }
            }
            drop(tx);

            write_handle.await.map_err(std::io::Error::other)??;
            Ok::<(), DownloadError>(())
        }));
    }

    for handle in handles {
        handle.await.map_err(std::io::Error::other)??;
    }

    // Final integrity check
    let mut hasher = Hasher::new();
    let mut file = std::fs::File::open(dest)?;
    std::io::copy(&mut file, &mut hasher)?;
    let actual_hash = hasher.finalize().to_hex().to_string();

    if actual_hash != expected_hash {
        if let Some(rep) = reporter {
            rep.failed(pkg_name, &format_size(total_size), "hash mismatch");
        }
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

/// Simultaneously downloads, caches, and extracts an archive via a streaming pipeline.
pub async fn download_and_extract<R: Reporter + Clone + 'static>(
    client: &Client,
    pkg_name: &str,
    version: &str,
    url: &str,
    cache_dest: &Path,
    extract_dest: &Path,
    expected_hash: &str,
    reporter: &R,
) -> Result<String, DownloadError> {
    use async_compression::tokio::bufread::ZstdDecoder;
    use tokio_tar::Archive;
    use tokio_util::io::StreamReader;

    let user_agent = crate::USER_AGENT;

    let head_resp = client
        .head(url)
        .header(reqwest::header::USER_AGENT, user_agent)
        .send()
        .await?;

    let total_size = head_resp.content_length().unwrap_or(0);
    // Note: We ignore accept_ranges for pipelined extraction as it prefers sequential stream.

    reporter.downloading(pkg_name, version, 0, total_size);

    let response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, user_agent)
        .send()
        .await?
        .error_for_status()?;

    let mut stream = response.bytes_stream();
    let mut file = File::create(cache_dest).await?;
    let mut hasher = Hasher::new();
    let mut downloaded: u64 = 0;

    // Channel for Pipelined Extraction
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<bytes::Bytes, std::io::Error>>(32);
    let stream_reader = StreamReader::new(tokio_stream::wrappers::ReceiverStream::new(rx));

    let format = crate::io::extract::detect_format(Path::new(url));
    let is_gzip = format == crate::io::extract::ArchiveFormat::TarGz;
    let is_zip = format == crate::io::extract::ArchiveFormat::Zip;
    let is_raw = format == crate::io::extract::ArchiveFormat::RawBinary;

    // Fast path for non-tar formats (zip/raw)
    if is_zip || is_raw {
        drop(tx);
        return run_simple_download(
            stream,
            file,
            hasher,
            pkg_name,
            version,
            total_size,
            reporter,
            expected_hash,
            cache_dest,
            extract_dest,
            is_zip,
            is_raw,
        )
        .await;
    }

    // Spawn Extractor Task
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

    while let Some(chunk_res) = stream.next().await {
        let chunk = chunk_res?;
        file.write_all(&chunk).await?;
        hasher.write_all(&chunk)?;

        downloaded += chunk.len() as u64;
        reporter.downloading(pkg_name, version, downloaded, total_size);

        if tx.send(Ok(chunk)).await.is_err() {
            return Err(
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, "Extractor died").into(),
            );
        }
    }
    drop(tx);

    file.flush().await?;
    let actual_hash = hasher.finalize().to_hex().to_string();

    if actual_hash != expected_hash {
        reporter.failed(pkg_name, &format_size(downloaded), "hash mismatch");
        tokio::fs::remove_file(cache_dest).await.ok();
        return Err(DownloadError::HashMismatch {
            expected: expected_hash.to_string(),
            actual: actual_hash,
        });
    }

    match extractor_handle.await {
        Ok(Ok(_)) => Ok(actual_hash),
        Ok(Err(e)) => Err(DownloadError::Io(e)),
        Err(e) => Err(DownloadError::Io(std::io::Error::other(e))),
    }
}

async fn run_simple_download<R: Reporter>(
    mut stream: impl Unpin + futures::Stream<Item = reqwest::Result<bytes::Bytes>>,
    mut file: File,
    mut hasher: Hasher,
    pkg_name: &str,
    version: &str,
    total_size: u64,
    reporter: &R,
    expected_hash: &str,
    cache_dest: &Path,
    extract_dest: &Path,
    is_zip: bool,
    is_raw: bool,
) -> Result<String, DownloadError> {
    let mut downloaded = 0;
    while let Some(chunk_res) = stream.next().await {
        let chunk = chunk_res?;
        file.write_all(&chunk).await?;
        hasher.write_all(&chunk)?;
        downloaded += chunk.len() as u64;
        reporter.downloading(pkg_name, version, downloaded, total_size);
    }
    file.flush().await?;

    let actual_hash = hasher.finalize().to_hex().to_string();
    if actual_hash != expected_hash {
        reporter.failed(pkg_name, &format_size(downloaded), "hash mismatch");
        tokio::fs::remove_file(cache_dest).await.ok();
        return Err(DownloadError::HashMismatch {
            expected: expected_hash.to_string(),
            actual: actual_hash,
        });
    }

    if is_zip {
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
    } else if is_raw {
        let dest_path = extract_dest.join(pkg_name);
        tokio::fs::copy(cache_dest, &dest_path).await?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = tokio::fs::metadata(&dest_path).await?.permissions();
            perms.set_mode(0o755);
            tokio::fs::set_permissions(&dest_path, perms).await?;
        }
    }

    Ok(actual_hash)
}
