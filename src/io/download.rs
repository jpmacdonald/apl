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

use crate::ui::Output;

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
    version: &str,
    url: &str,
    dest: &Path,
    expected_hash: &str,
    output: &Output,
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
    output.downloading(pkg_name, version, 0, total_size);

    if total_size > 10 * 1024 * 1024 && accept_ranges {
        return download_chunked(
            client,
            pkg_name,
            version,
            url,
            dest,
            expected_hash,
            total_size,
            Some(output),
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
        output.downloading(pkg_name, version, downloaded, total_size);
    }

    file.flush().await?;
    let actual_hash = hasher.finalize().to_hex().to_string();

    if actual_hash != expected_hash {
        output.failed(pkg_name, &format_size(downloaded), "hash mismatch");
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
        return download_chunked(
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
/// Download a file in chunks parallelly
async fn download_chunked(
    client: &Client,
    pkg_name: &str,
    version: &str,
    url: &str,
    dest: &Path,
    expected_hash: &str,
    total_size: u64,
    output: Option<&Output>,
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
        let output = output.cloned(); // Option<Output> is cheap to clone if Output is Arc-like, but Output contains sender.
        // Sender is cloneable.
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

                if let Some(out) = &output {
                    let mut d = downloaded.lock().await;
                    *d += len;
                    out.downloading(&pkg_name, &version, *d, total_size);
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

    let mut hasher = Hasher::new();
    let mut file = std::fs::File::open(dest)?;
    std::io::copy(&mut file, &mut hasher)?;
    let actual_hash = hasher.finalize().to_hex().to_string();

    if actual_hash != expected_hash {
        if let Some(out) = output {
            // We can't access total_size easily if we don't have downloaded count lock here?
            // But we have total_size arg.
            out.failed(pkg_name, &format_size(total_size), "hash mismatch");
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
    version: &str,
    url: &str,
    cache_dest: &Path,
    extract_dest: &Path,
    expected_hash: &str,
    output: &Output,
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

    output.downloading(pkg_name, version, 0, total_size);

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
    let format = crate::io::extract::detect_format(Path::new(url));
    let is_gzip = format == crate::io::extract::ArchiveFormat::TarGz;
    let is_zip = format == crate::io::extract::ArchiveFormat::Zip;
    let is_raw = format == crate::io::extract::ArchiveFormat::RawBinary;

    // If simple download is sufficient (zip/raw), just run that
    if is_zip || is_raw {
        drop(tx);
        return run_simple_download(
            stream,
            file,
            hasher,
            pkg_name,
            version,
            total_size,
            output,
            expected_hash,
            cache_dest,
            extract_dest,
            is_zip,
            is_raw,
        )
        .await;
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
        output.downloading(pkg_name, "", downloaded, total_size);

        // Pipe to Extractor (Clone bytes is cheap, it's Arc)
        if tx.send(Ok(chunk)).await.is_err() {
            return Err(
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, "Extractor died").into(),
            );
        }
    }
    drop(tx); // Close channel

    file.flush().await?;
    let actual_hash = hasher.finalize().to_hex().to_string();

    if actual_hash != expected_hash {
        output.failed(pkg_name, &format_size(downloaded), "hash mismatch");
        tokio::fs::remove_file(cache_dest).await.ok();
        return Err(DownloadError::HashMismatch {
            expected: expected_hash.to_string(),
            actual: actual_hash,
        });
    }

    // Wait for extraction to finish
    match extractor_handle.await {
        Ok(Ok(_)) => Ok(actual_hash),
        Ok(Err(e)) => Err(DownloadError::Io(e)),
        Err(e) => Err(DownloadError::Io(std::io::Error::other(e))),
    }
}

async fn run_simple_download(
    mut stream: impl Unpin + futures::Stream<Item = reqwest::Result<bytes::Bytes>>,
    mut file: File,
    mut hasher: Hasher,
    pkg_name: &str,
    version: &str,
    total_size: u64,
    output: &Output,
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
        output.downloading(pkg_name, version, downloaded, total_size);
    }
    file.flush().await?;

    let actual_hash = hasher.finalize().to_hex().to_string();
    if actual_hash != expected_hash {
        output.failed(pkg_name, &format_size(downloaded), "hash mismatch");
        tokio::fs::remove_file(cache_dest).await.ok();
        return Err(DownloadError::HashMismatch {
            expected: expected_hash.to_string(),
            actual: actual_hash,
        });
    }

    if is_zip {
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
    } else if is_raw {
        // For raw binaries, we rename it to the package name for easy linking
        let dest_path = extract_dest.join(pkg_name);
        tokio::fs::copy(cache_dest, &dest_path).await?;

        // Ensure executable permissions
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

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn test_download_and_verify() {
        // Note: This test requires network access and a known correct hash
        // Skipping for now as it depends on external service
    }
}
