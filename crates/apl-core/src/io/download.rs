//! Async download and verification module supporting parallel chunking and progress reporting.
//!
//! Handles file downloads with streaming SHA256 verification.

use std::io::Write;
use std::path::Path;
use std::sync::Arc;

use futures::StreamExt;
use reqwest::Client;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

use crate::Reporter;
use crate::types::{ArtifactFormat, PackageName, Version};

#[derive(Error, Debug)]
pub enum DownloadError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Hash mismatch: expected {expected}, got {actual}")]
    HashMismatch { expected: String, actual: String },
}

/// Request for a download operation
pub struct DownloadRequest<'a, R: Reporter + Clone + 'static> {
    pub client: &'a Client,
    pub pkg_name: &'a PackageName,
    pub version: &'a Version,
    pub url: &'a str,
    pub dest: &'a Path,
    pub expected_hash: &'a str,
    pub reporter: &'a R,
    pub extract_dest: Option<&'a Path>,
}

impl<'a, R: Reporter + Clone + 'static> DownloadRequest<'a, R> {
    pub fn new(
        client: &'a Client,
        pkg_name: &'a PackageName,
        version: &'a Version,
        url: &'a str,
        dest: &'a Path,
        expected_hash: &'a str,
        reporter: &'a R,
    ) -> Self {
        Self {
            client,
            pkg_name,
            version,
            url,
            dest,
            expected_hash,
            reporter,
            extract_dest: None,
        }
    }

    pub fn with_extract_dest(mut self, extract_dest: &'a Path) -> Self {
        self.extract_dest = Some(extract_dest);
        self
    }

    /// Execute the download (and extraction if requested)
    pub async fn execute(self) -> Result<String, DownloadError> {
        if self.extract_dest.is_some() {
            download_and_extract(self).await
        } else {
            download_and_verify_mp(self).await
        }
    }
}

/// Downloads and verifies a file, automatically switching to parallel chunking for large files.
pub async fn download_and_verify_mp<R: Reporter + Clone + 'static>(
    req: DownloadRequest<'_, R>,
) -> Result<String, DownloadError> {
    let client = req.client;
    let pkg_name = req.pkg_name;
    let version = req.version;
    let url = req.url;
    let dest = req.dest;
    let expected_hash = req.expected_hash;
    let reporter = req.reporter;

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
    reporter.downloading(pkg_name, version, 0, Some(total_size));

    // Detect if this is a manifest-based chunked download
    if url.contains("/manifests/") {
        return download_from_manifest(req).await;
    }

    if total_size > 10 * 1024 * 1024 && accept_ranges {
        let opts = ChunkedDownloadOptions {
            pkg_name: Some(pkg_name),
            version: Some(version),
            url,
            dest,
            expected_hash,
            total_size,
            reporter: Some(reporter.clone()),
            user_agent,
        };
        return download_chunked(client, opts).await;
    }

    let response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, user_agent)
        .send()
        .await?
        .error_for_status()?;

    let mut file = File::create(dest).await?;
    let mut stream = response.bytes_stream();
    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        hasher.write_all(&chunk)?;
        downloaded += chunk.len() as u64;
        reporter.downloading(pkg_name, version, downloaded, Some(total_size));
    }

    file.flush().await?;
    let actual_hash = hex::encode(hasher.finalize());

    if actual_hash != expected_hash {
        reporter.failed(pkg_name, version, "hash mismatch");
        tokio::fs::remove_file(dest).await.ok();
        return Err(DownloadError::HashMismatch {
            expected: expected_hash.to_string(),
            actual: actual_hash,
        });
    }

    Ok(actual_hash)
}

/// Downloads a manifest and reassembles the blob from chunks.
async fn download_from_manifest<R: Reporter + Clone + 'static>(
    req: DownloadRequest<'_, R>,
) -> Result<String, DownloadError> {
    use crate::io::chunked::BlobManifest;
    use tokio::sync::Semaphore;

    let client = req.client;
    let url = req.url;
    let dest = req.dest;
    let reporter = req.reporter;
    let pkg_name = req.pkg_name;
    let version = req.version;
    let expected_hash = req.expected_hash;

    // 1. Fetch Manifest
    let resp = client.get(url).send().await?.error_for_status()?;
    let manifest_json = resp.text().await?;

    let manifest = BlobManifest::from_json(&manifest_json)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    let total_size = manifest.size;
    reporter.downloading(pkg_name, version, 0, Some(total_size));

    // 2. Prepare for chunk downloads
    let base_url = url.split("/manifests/").next().unwrap_or_default();
    let downloaded = Arc::new(tokio::sync::Mutex::new(0u64));
    let semaphore = Arc::new(Semaphore::new(16)); // Max 16 concurrent chunk downloads
    let mut handles = Vec::new();

    // We store chunks in memory or temp files?
    // For small/medium blobs, memory is fine. For large ones, we should use a temp store.
    // Let's use memory for now as a baseline.
    let chunk_map = Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new()));

    for chunk_ref in manifest.chunks.clone() {
        let chunk_hash = chunk_ref.hash.as_str().to_string();
        let chunk_url = format!("{base_url}/cas/{chunk_hash}");
        let client = client.clone();
        let chunk_map = chunk_map.clone();
        let semaphore = semaphore.clone();
        let downloaded = downloaded.clone();
        let reporter = reporter.clone();
        let pkg_name = pkg_name.clone();
        let version = version.clone();

        handles.push(tokio::spawn(async move {
            let _permit = semaphore
                .acquire()
                .await
                .map_err(|_| std::io::Error::other("Semaphore closed"))?;

            let resp = client.get(&chunk_url).send().await?.error_for_status()?;
            let data = resp.bytes().await?;

            let len = data.len() as u64;
            chunk_map.lock().await.insert(chunk_hash, data.to_vec());

            let mut d = downloaded.lock().await;
            *d += len;
            reporter.downloading(&pkg_name, &version, *d, Some(total_size));

            Ok::<(), DownloadError>(())
        }));
    }

    // Wait for all chunks
    for handle in handles {
        handle.await.map_err(std::io::Error::other)??;
    }

    // 3. Reassemble
    let final_chunk_map = Arc::try_unwrap(chunk_map).unwrap().into_inner();
    let reassembled = crate::io::chunked::reassemble(&manifest, &final_chunk_map)
        .ok_or_else(|| std::io::Error::other("Failed to reassemble blob from chunks"))?;

    // 4. Verify original hash
    let mut hasher = Sha256::new();
    hasher.update(&reassembled);
    let actual_hash = hex::encode(hasher.finalize());

    if actual_hash != expected_hash {
        reporter.failed(pkg_name, version, "hash mismatch");
        return Err(DownloadError::HashMismatch {
            expected: expected_hash.to_string(),
            actual: actual_hash,
        });
    }

    // 5. Write to destination
    tokio::fs::write(dest, reassembled).await?;

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
        let opts = ChunkedDownloadOptions {
            pkg_name: None,
            version: None,
            url,
            dest,
            expected_hash,
            total_size,
            reporter: None::<crate::NullReporter>,
            user_agent,
        };
        return download_chunked(client, opts).await;
    }

    let response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, user_agent)
        .send()
        .await?
        .error_for_status()?;

    let mut file = File::create(dest).await?;
    let mut stream = response.bytes_stream();
    let mut hasher = Sha256::new();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        hasher.update(&chunk);
    }

    file.flush().await?;
    let actual_hash = hex::encode(hasher.finalize());

    if actual_hash != expected_hash {
        tokio::fs::remove_file(dest).await.ok();
        return Err(DownloadError::HashMismatch {
            expected: expected_hash.to_string(),
            actual: actual_hash,
        });
    }

    Ok(actual_hash)
}

struct ChunkedDownloadOptions<'a, R: Reporter> {
    pkg_name: Option<&'a PackageName>,
    version: Option<&'a Version>,
    url: &'a str,
    dest: &'a Path,
    expected_hash: &'a str,
    total_size: u64,
    reporter: Option<R>,
    user_agent: &'a str,
}

/// Executes a concurrent, chunked download using HTTP Range headers.
async fn download_chunked<R: Reporter + Clone + 'static>(
    client: &Client,
    opts: ChunkedDownloadOptions<'_, R>,
) -> Result<String, DownloadError> {
    let total_size = opts.total_size;
    let pkg_name = opts.pkg_name;
    let version = opts.version;
    let url = opts.url;
    let dest = opts.dest;
    let expected_hash = opts.expected_hash;
    let reporter = opts.reporter;
    let user_agent = opts.user_agent;

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
        let pkg_name = pkg_name.cloned();
        let version = version.cloned();

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

                if let (Some(rep), Some(name), Some(ver)) = (&reporter, &pkg_name, &version) {
                    let mut d = downloaded.lock().await;
                    *d += len;
                    rep.downloading(name, ver, *d, Some(total_size));
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
    // Final integrity check in a blocking thread
    let dest_clone = dest.to_path_buf();
    let actual_hash = tokio::task::spawn_blocking(move || {
        let mut hasher = Sha256::new();
        let mut file = std::fs::File::open(&dest_clone)?;
        let mut buffer = [0u8; 8192];
        use std::io::Read;
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }
        Ok::<String, std::io::Error>(hex::encode(hasher.finalize()))
    })
    .await
    .map_err(std::io::Error::other)??;

    if actual_hash != expected_hash {
        if let (Some(rep), Some(name), Some(ver)) = (reporter, pkg_name, version) {
            rep.failed(name, ver, "hash mismatch");
        }
        let _ = tokio::fs::remove_file(dest).await;
        return Err(DownloadError::HashMismatch {
            expected: expected_hash.to_string(),
            actual: actual_hash,
        });
    }

    Ok(actual_hash)
}

/// Simultaneously downloads, caches, and extracts an archive via a streaming pipeline.
pub async fn download_and_extract<R: Reporter + Clone + 'static>(
    req: DownloadRequest<'_, R>,
) -> Result<String, DownloadError> {
    use async_compression::tokio::bufread::ZstdDecoder;
    use tokio_tar::Archive;
    use tokio_util::io::StreamReader;

    let client = req.client;
    let pkg_name = req.pkg_name;
    let version = req.version;
    let url = req.url;
    let cache_dest = req.dest;
    let extract_dest = req.extract_dest.ok_or_else(|| {
        DownloadError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Missing extract dest",
        ))
    })?;
    let expected_hash = req.expected_hash;
    let reporter = req.reporter;

    let user_agent = crate::USER_AGENT;

    let head_resp = client
        .head(url)
        .header(reqwest::header::USER_AGENT, user_agent)
        .send()
        .await?;

    let total_size = head_resp.content_length().unwrap_or(0);
    // Note: We ignore accept_ranges for pipelined extraction as it prefers sequential stream.

    reporter.downloading(pkg_name, version, 0, Some(total_size));

    let response = client
        .get(url)
        .header(reqwest::header::USER_AGENT, user_agent)
        .send()
        .await?
        .error_for_status()?;

    let mut stream = response.bytes_stream();
    let mut file = File::create(cache_dest).await?;
    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;

    // Channel for Pipelined Extraction
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<bytes::Bytes, std::io::Error>>(32);
    let stream_reader = StreamReader::new(tokio_stream::wrappers::ReceiverStream::new(rx));

    let format = crate::io::extract::detect_format(Path::new(url));
    let is_gzip = format == ArtifactFormat::TarGz;
    let is_zip = format == ArtifactFormat::Zip;
    let is_pkg = format == ArtifactFormat::Pkg;
    let is_raw = format == ArtifactFormat::Binary;

    // Fast path for non-tar formats (zip/raw/pkg)
    if is_zip || is_raw || is_pkg {
        drop(tx);
        let opts = SimpleDownloadOptions {
            pkg_name,
            version,
            total_size,
            reporter,
            expected_hash,
            cache_dest,
            extract_dest,
            format,
        };
        return run_simple_download(stream, file, hasher, opts).await;
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
        reporter.downloading(pkg_name, version, downloaded, Some(total_size));

        if tx.send(Ok(chunk)).await.is_err() {
            return Err(
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, "Extractor died").into(),
            );
        }
    }
    drop(tx);

    file.flush().await?;
    let actual_hash = hex::encode(hasher.finalize());

    if actual_hash != expected_hash {
        reporter.failed(pkg_name, version, "hash mismatch");
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

struct SimpleDownloadOptions<'a, R: Reporter> {
    pkg_name: &'a PackageName,
    version: &'a Version,
    total_size: u64,
    reporter: &'a R,
    expected_hash: &'a str,
    cache_dest: &'a Path,
    extract_dest: &'a Path,
    format: ArtifactFormat,
}

async fn run_simple_download<R: Reporter>(
    mut stream: impl Unpin + futures::Stream<Item = reqwest::Result<bytes::Bytes>>,
    mut file: File,
    mut hasher: Sha256,
    opts: SimpleDownloadOptions<'_, R>,
) -> Result<String, DownloadError> {
    let mut downloaded = 0;
    while let Some(chunk_res) = stream.next().await {
        let chunk = chunk_res?;
        file.write_all(&chunk).await?;
        hasher.write_all(&chunk)?;
        downloaded += chunk.len() as u64;
        opts.reporter.downloading(
            opts.pkg_name,
            opts.version,
            downloaded,
            Some(opts.total_size),
        );
    }
    file.flush().await?;

    let actual_hash = hex::encode(hasher.finalize());
    if actual_hash != opts.expected_hash {
        opts.reporter
            .failed(opts.pkg_name, opts.version, "hash mismatch");
        tokio::fs::remove_file(opts.cache_dest).await.ok();
        return Err(DownloadError::HashMismatch {
            expected: opts.expected_hash.to_string(),
            actual: actual_hash,
        });
    }

    // format is already imported from types

    match opts.format {
        ArtifactFormat::Zip => {
            let cache_path = opts.cache_dest.to_path_buf();
            let extract_path = opts.extract_dest.to_path_buf();
            tokio::task::spawn_blocking(move || {
                let file = std::fs::File::open(&cache_path)?;
                let mut archive = zip::ZipArchive::new(file)?;
                archive.extract(&extract_path)?;
                Ok::<(), std::io::Error>(())
            })
            .await
            .map_err(std::io::Error::other)??;
        }
        ArtifactFormat::Binary => {
            let dest_path = opts.extract_dest.join(opts.pkg_name.as_str());
            tokio::fs::copy(opts.cache_dest, &dest_path).await?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mut perms = tokio::fs::metadata(&dest_path).await?.permissions();
                perms.set_mode(0o755);
                tokio::fs::set_permissions(&dest_path, perms).await?;
            }
        }
        ArtifactFormat::Pkg => {
            let cache_path = opts.cache_dest.to_path_buf();
            let extract_path = opts.extract_dest.to_path_buf();
            tokio::task::spawn_blocking(move || {
                crate::io::extract::extract_pkg(&cache_path, &extract_path)
                    .map_err(std::io::Error::other)?;
                Ok::<(), std::io::Error>(())
            })
            .await
            .map_err(std::io::Error::other)??;
        }
        _ => {}
    }

    Ok(actual_hash)
}
