//! Artifact discovery strategies for upstream package sources.
//!
//! Each strategy knows how to fetch version and artifact metadata from a
//! specific upstream provider (e.g. HashiCorp, Go, Node.js, GitHub releases)
//! and return a list of `Artifact` records suitable for indexing.

use anyhow::Result;
use apl_schema::Artifact;
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use futures::stream::{self, StreamExt};
use regex::Regex;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use tokio::process::Command;

/// Cached HTTP response metadata (`ETag` and Last-Modified) for a single URL,
/// used to avoid redundant downloads when the upstream has not changed.
#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct CacheEntry {
    /// HTTP `ETag` header value from the last successful response.
    pub etag: Option<String>,
    /// HTTP `Last-Modified` header value from the last successful response.
    pub last_modified: Option<String>,
}

/// A map from URL strings to their corresponding `CacheEntry`, used by
/// strategies to persist HTTP caching state across runs.
pub type StrategyCache = HashMap<String, CacheEntry>;

/// Trait implemented by every artifact discovery strategy.
///
/// A strategy knows how to contact an upstream source, enumerate available
/// versions, and produce `Artifact` records for each platform-specific
/// downloadable asset.
#[async_trait]
pub trait Strategy: Send + Sync {
    /// Fetch artifacts based on the configuration strategy.
    ///
    /// `known_versions` contains versions already in the index - strategies should
    /// skip these to avoid redundant work. This enables incremental indexing.
    ///
    /// `cache` contains ETag/Last-Modified data for upstream URLs.
    ///
    /// # Errors
    ///
    /// Returns an error if the upstream source cannot be reached or if the
    /// response cannot be parsed into valid artifact metadata.
    async fn fetch_artifacts(
        &self,
        known_versions: &HashSet<String>,
        cache: &mut StrategyCache,
    ) -> Result<Vec<Artifact>>;
}

/// Helper to fetch text with HTTP caching (ETag/Last-Modified).
/// Returns Ok(None) if 304 Not Modified.
async fn fetch_text_with_cache(
    client: &Client,
    url: &str,
    cache: &mut StrategyCache,
) -> Result<Option<String>> {
    let mut req = client.get(url);

    if let Some(entry) = cache.get(url) {
        if let Some(etag) = &entry.etag {
            req = req.header("If-None-Match", etag);
        }
        if let Some(lm) = &entry.last_modified {
            req = req.header("If-Modified-Since", lm);
        }
    }

    let resp = req.send().await?.error_for_status()?;

    if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
        println!("    [Cache] {url} not modified");
        return Ok(None);
    }

    // Update cache
    let etag = resp
        .headers()
        .get("etag")
        .and_then(|h| h.to_str().ok().map(std::string::ToString::to_string));
    let last_modified = resp
        .headers()
        .get("last-modified")
        .and_then(|h| h.to_str().ok().map(std::string::ToString::to_string));

    if etag.is_some() || last_modified.is_some() {
        cache.insert(
            url.to_string(),
            CacheEntry {
                etag,
                last_modified,
            },
        );
    }

    let text = resp.text().await?;
    Ok(Some(text))
}

/// Helper to stream-download and compute SHA256 (expensive but accurate)
async fn download_and_hash(client: &Client, url: &str) -> Result<String> {
    println!("    [Hashing] Downloading {url}...");
    let mut resp = client.get(url).send().await?.error_for_status()?;
    let mut hasher = Sha256::new();
    while let Some(chunk) = resp.chunk().await? {
        hasher.update(&chunk);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Helper to fetch SHASUMS text file and parse into filename -> sha256 map
async fn fetch_shasums(client: &Client, url: &str) -> Result<HashMap<String, String>> {
    println!("    [Hashing] Fetching checksums from {url}...");
    let text = client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let mut map = HashMap::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            let sha = parts[0].to_string();
            // Filename might be "*filename" or just "filename"
            let filename = parts[1].trim_start_matches('*').to_string();
            map.insert(filename, sha);
        }
    }
    Ok(map)
}

/// Strategy for discovering artifacts from the `HashiCorp` release index.
///
/// Fetches the JSON release index for a given `HashiCorp` product, filters for
/// macOS builds, and retrieves SHA-256 checksums from the accompanying
/// `SHA256SUMS` files.
#[derive(Debug)]
pub struct HashiCorpStrategy {
    product: String,
    client: Client,
}

impl HashiCorpStrategy {
    /// Create a new `HashiCorpStrategy` for the given product name
    /// (e.g. `"terraform"`, `"vault"`).
    pub fn new(product: String) -> Self {
        Self {
            product,
            client: Client::new(),
        }
    }
}

#[derive(Deserialize)]
struct HashiCorpIndex {
    versions: std::collections::HashMap<String, VersionData>,
}

#[derive(Deserialize)]
struct VersionData {
    builds: Vec<BuildData>,
}

#[derive(Deserialize)]
struct BuildData {
    arch: String,
    os: String,
    url: String,
    filename: String,
}

#[async_trait]
impl Strategy for HashiCorpStrategy {
    async fn fetch_artifacts(
        &self,
        known_versions: &HashSet<String>,
        cache: &mut StrategyCache,
    ) -> Result<Vec<Artifact>> {
        let url = format!("https://releases.hashicorp.com/{}/index.json", self.product);

        // HashiCorp API supports ETags? Let's try or just assume it does.
        // If not, it falls back to 200.
        // For JSON, we use a similar helper or just inline it?
        // Let's implement fetch_json_with_cache if needed, or just manual here for now.

        let mut req = self.client.get(&url);
        if let Some(entry) = cache.get(&url) {
            if let Some(etag) = &entry.etag {
                req = req.header("If-None-Match", etag);
            }
        }

        let resp = req.send().await?.error_for_status()?;
        if resp.status() == reqwest::StatusCode::NOT_MODIFIED {
            println!("  [Cache] HashiCorp index not modified");
            return Ok(vec![]);
        }

        // Update ETag
        if let Some(etag) = resp
            .headers()
            .get("etag")
            .and_then(|h| h.to_str().ok().map(std::string::ToString::to_string))
        {
            cache.insert(
                url.clone(),
                CacheEntry {
                    etag: Some(etag),
                    last_modified: None,
                },
            );
        }

        let resp: HashiCorpIndex = resp.json().await?;

        let resp_len = resp.versions.len();
        let mut artifacts = Vec::new();
        // let mut skipped = 0;

        // Filter and collect tasks
        let pending_versions: Vec<_> = resp
            .versions
            .into_iter()
            .filter(|(version, _)| !version.contains('-') && !known_versions.contains(version))
            .collect();

        let skipped_count = resp_len - pending_versions.len();

        // Process in parallel (concurrency: 10)
        let mut stream = stream::iter(pending_versions)
            .map(|(version, data)| {
                let client = self.client.clone();
                let product = self.product.clone();
                async move {
                    // Fetch SHA256SUMS
                    let sha_url = format!(
                        "https://releases.hashicorp.com/{product}/{version}/{product}_{version}_SHA256SUMS"
                    );

                    let shas = fetch_shasums(&client, &sha_url).await.unwrap_or_default();

                    let mut version_artifacts = Vec::new();
                    // APL currently supports:
                    let allowed_platforms = [
                        ("darwin", "amd64", "x86_64-apple-darwin"),
                        ("darwin", "arm64", "aarch64-apple-darwin"),
                    ];

                    for build in data.builds {
                        for (os, arch, apl_arch) in allowed_platforms {
                            if build.os == os && build.arch == arch {
                                if let Some(checksum) = shas.get(&build.filename) {
                                    if !checksum.is_empty() {
                                        version_artifacts.push(Artifact {
                                            name: product.clone(),
                                            version: version.clone(),
                                            arch: apl_arch.to_string(),
                                            url: build.url.clone(),
                                            sha256: checksum.clone(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                    version_artifacts
                }
            })
            .buffer_unordered(10);

        while let Some(mut batch) = stream.next().await {
            artifacts.append(&mut batch);
        }

        if skipped_count > 0 {
            // We can't easily track exact skipped count due to filter logic,
            // but we can infer it or just be generic.
            // Let's just say "Skipped known versions" if we have logic to track it
            println!("  [Incremental] Skipped known/unstable versions");
        }

        Ok(artifacts)
    }
}

// ... HashiCorp implementation ...

/// Strategy for discovering Go toolchain artifacts from the official Go
/// download API.
#[derive(Debug)]
pub struct GolangStrategy;

/// Strategy for discovering Node.js artifacts from the official release
/// index.
#[derive(Debug)]
pub struct NodeStrategy;

#[derive(Deserialize)]
struct GoRelease {
    version: String,
    files: Vec<GoFile>,
}

#[derive(Deserialize)]
struct GoFile {
    os: String,
    arch: String,
    filename: String,
    // url is not in json, must construct
    sha256: String,
}

#[async_trait]
impl Strategy for GolangStrategy {
    async fn fetch_artifacts(
        &self,
        known_versions: &HashSet<String>,
        _cache: &mut StrategyCache,
    ) -> Result<Vec<Artifact>> {
        let url = "https://go.dev/dl/?mode=json&include=all"; // fetch all versions
        let releases = reqwest::get(url).await?.json::<Vec<GoRelease>>().await?;

        let mut artifacts = Vec::new();
        let mut skipped = 0;

        for release in releases {
            let version = release.version.trim_start_matches("go").to_string();

            // Skip already-indexed versions
            if known_versions.contains(&version) {
                skipped += 1;
                continue;
            }
            // APL supported architectures
            for file in release.files {
                let (apl_arch, valid) = match (file.os.as_str(), file.arch.as_str()) {
                    ("darwin", "amd64") => ("x86_64-apple-darwin", true),
                    ("darwin", "arm64") => ("aarch64-apple-darwin", true),
                    _ => ("", false),
                };

                if valid {
                    if file.sha256.is_empty() {
                        eprintln!(
                            "    [WARN] Skipping go v{version} ({apl_arch}) - Missing SHA256"
                        );
                        continue;
                    }

                    artifacts.push(Artifact {
                        name: "go".to_string(),
                        version: version.clone(),
                        arch: apl_arch.to_string(),
                        url: format!("https://go.dev/dl/{}", file.filename),
                        sha256: file.sha256,
                    });
                }
            }
        }
        if skipped > 0 {
            println!("  [Incremental] Skipped {skipped} known versions");
        }
        println!("  Found {} Go artifacts", artifacts.len());
        Ok(artifacts)
    }
}

#[derive(Deserialize)]
struct NodeRelease {
    version: String,
    files: Vec<String>,
    // nodejs.org/dist/index.json doesn't have checksums, need SHASUMS256.txt?
    // Actually the generate.py used index.json which has files list but not SHAs?
    // Checking previous python: it parsed index.json for versions, then checked SHASUMS256.txt
}

#[async_trait]
impl Strategy for NodeStrategy {
    async fn fetch_artifacts(
        &self,
        known_versions: &HashSet<String>,
        _cache: &mut StrategyCache,
    ) -> Result<Vec<Artifact>> {
        let url = "https://nodejs.org/dist/index.json";
        let releases = reqwest::get(url).await?.json::<Vec<NodeRelease>>().await?;

        let mut artifacts = Vec::new();
        // let mut skipped = 0;
        // let mut processed = 0;

        let total_count = releases.len();

        // Filter pending
        let pending_releases: Vec<_> = releases
            .into_iter()
            .filter(|r| {
                let v = r.version.trim_start_matches('v');
                !known_versions.contains(v)
            })
            .take(20) // Limit processed
            .collect();

        let skipped_count = total_count - pending_releases.len();

        if pending_releases.is_empty() {
            println!("  [Incremental] All versions already indexed");
            return Ok(vec![]);
        }

        // Parallel fetch
        let mut stream = stream::iter(pending_releases)
            .map(|release| async move {
                let version = release.version.trim_start_matches('v').to_string();
                let sha_url = format!("https://nodejs.org/dist/{}/SHASUMS256.txt", release.version);
                let shas = fetch_shasums(&Client::new(), &sha_url)
                    .await
                    .unwrap_or_default();

                let mut batch = Vec::new();
                for file in &release.files {
                    let (apl_arch, suffix) = match file.as_str() {
                        "osx-arm64-tar" => ("aarch64-apple-darwin", "darwin-arm64.tar.gz"),
                        "osx-x64-tar" => ("x86_64-apple-darwin", "darwin-x64.tar.gz"),
                        _ => continue,
                    };

                    let filename = format!("node-{}-{}", release.version, suffix);
                    if let Some(checksum) = shas.get(&filename) {
                        if !checksum.is_empty() {
                            batch.push(Artifact {
                                name: "node".to_string(),
                                version: version.clone(),
                                arch: apl_arch.to_string(),
                                url: format!(
                                    "https://nodejs.org/dist/{}/{}",
                                    release.version, filename
                                ),
                                sha256: checksum.clone(),
                            });
                        }
                    }
                }
                batch
            })
            .buffer_unordered(10);

        while let Some(mut batch) = stream.next().await {
            artifacts.append(&mut batch);
        }
        if skipped_count > 0 {
            println!("  [Incremental] Skipped {skipped_count} known versions");
        }
        println!("  Found {} Node.js artifacts", artifacts.len());
        Ok(artifacts)
    }
}

/// Strategy for discovering AWS CLI v2 artifacts by parsing the upstream
/// changelog and downloading macOS `.pkg` installers to compute their
/// SHA-256 checksums.
#[derive(Debug)]
pub struct AwsStrategy;

#[async_trait]
impl Strategy for AwsStrategy {
    async fn fetch_artifacts(
        &self,
        known_versions: &HashSet<String>,
        cache: &mut StrategyCache,
    ) -> Result<Vec<Artifact>> {
        // AWS CLI v2 Changelog parsing
        let url = "https://raw.githubusercontent.com/aws/aws-cli/v2/CHANGELOG.rst";

        let Some(text) = fetch_text_with_cache(&Client::new(), url, cache).await? else {
            println!("  [Cache] AWS Changelog not modified");
            return Ok(vec![]);
        };
        let mut artifacts = Vec::new();

        let re = Regex::new(r"(\d+\.\d+\.\d+)").unwrap();

        let mut versions = Vec::new();
        for line in text.lines() {
            if let Some(cap) = re.captures(line) {
                if line.trim() == &cap[1] {
                    versions.push(cap[1].to_string());
                }
            }
        }

        versions.sort_by(|a, b| b.cmp(a));
        versions.dedup();

        // Filter to only new versions
        let new_versions: Vec<_> = versions
            .into_iter()
            .filter(|v| !known_versions.contains(v))
            .take(5)
            .collect();

        if new_versions.is_empty() {
            println!("  [Incremental] All versions already indexed");
            return Ok(vec![]);
        }

        // Parallel hash
        let mut stream = stream::iter(new_versions)
            .map(|version| async move {
                let url = format!("https://awscli.amazonaws.com/AWSCLIV2-{version}.pkg");
                let sha = download_and_hash(&Client::new(), &url)
                    .await
                    .unwrap_or_else(|e| {
                        eprintln!("    [WARN] Failed to hash {url}: {e}");
                        String::new()
                    });

                if sha.is_empty() {
                    None
                } else {
                    Some(Artifact {
                        name: "aws".to_string(),
                        version,
                        arch: "universal-apple-darwin".to_string(),
                        url,
                        sha256: sha,
                    })
                }
            })
            .buffer_unordered(5); // Limit concurrency for large downloads

        while let Some(art_opt) = stream.next().await {
            if let Some(art) = art_opt {
                artifacts.push(art);
            }
        }
        println!("  Found {} AWS artifacts", artifacts.len());
        Ok(artifacts)
    }
}

/// Strategy for discovering `CPython` source tarballs from the official
/// Python FTP mirror.
#[derive(Debug)]
pub struct PythonStrategy;

#[async_trait]
impl Strategy for PythonStrategy {
    async fn fetch_artifacts(
        &self,
        known_versions: &HashSet<String>,
        cache: &mut StrategyCache,
    ) -> Result<Vec<Artifact>> {
        let url = "https://www.python.org/ftp/python/";
        let Some(text) = fetch_text_with_cache(&Client::new(), url, cache).await? else {
            println!("  [Cache] Python index not modified");
            return Ok(vec![]);
        };
        let re = Regex::new(r#"href="(\d+\.\d+\.\d+)/""#).unwrap();

        let mut versions: Vec<String> = re.captures_iter(&text).map(|c| c[1].to_string()).collect();
        versions.sort_by(|a, b| b.cmp(a));
        versions.dedup();

        // Filter to only new versions
        let new_versions: Vec<_> = versions
            .into_iter()
            .filter(|v| !known_versions.contains(v))
            .take(5)
            .collect();

        if new_versions.is_empty() {
            println!("  [Incremental] All versions already indexed");
            return Ok(vec![]);
        }

        let mut artifacts = Vec::new();
        let mut stream = stream::iter(new_versions)
            .map(|version| async move {
                let url =
                    format!("https://www.python.org/ftp/python/{version}/Python-{version}.tgz");
                let sha = download_and_hash(&Client::new(), &url)
                    .await
                    .unwrap_or_default();

                if sha.is_empty() {
                    None
                } else {
                    Some(Artifact {
                        name: "python".to_string(),
                        version,
                        arch: "universal-apple-darwin".to_string(),
                        url,
                        sha256: sha,
                    })
                }
            })
            .buffer_unordered(5);

        while let Some(art_opt) = stream.next().await {
            if let Some(art) = art_opt {
                artifacts.push(art);
            }
        }
        println!("  Found {} Python artifacts", artifacts.len());
        Ok(artifacts)
    }
}

/// Strategy for discovering Ruby source tarballs from the official Ruby
/// downloads page by scraping version numbers and their associated SHA-256
/// checksums.
#[derive(Debug)]
pub struct RubyStrategy;

#[async_trait]
impl Strategy for RubyStrategy {
    async fn fetch_artifacts(
        &self,
        known_versions: &HashSet<String>,
        cache: &mut StrategyCache,
    ) -> Result<Vec<Artifact>> {
        // https://www.ruby-lang.org/en/downloads/
        let url = "https://www.ruby-lang.org/en/downloads/";
        let Some(text) = fetch_text_with_cache(&Client::new(), url, cache).await? else {
            println!("  [Cache] Ruby index not modified");
            return Ok(vec![]);
        };

        // Regex to capture "Ruby 3.3.0" ... "sha256: <hash>"
        // This is fragile HTML parsing but typical for ruby-lang.
        // Pattern:
        // <td>Ruby 3.3.0</td>
        // ...
        // <td>...sha256: ...</td>
        // Actually the site format is:
        // <h3>Ruby 3.3.0</h3>
        // ...
        // <li>sha256: <code>...</code></li>

        // Let's use a simpler approach:
        // Find "Ruby X.Y.Z" and then look ahead for "sha256: <code>([a-f0-9]{64})</code>"

        // Or just capture strings that look like SHAs and map them?
        // No, need to link to version.

        // Assuming we can just find lines with "Ruby X.Y.Z" and nearby SHA.
        // Let's use stream-download as fallback if parsing fails?
        // No, ruby downloads are large.

        // Improved Regex: Look for the download link and the nearby checksum.
        // Link: href=".../ruby-3.3.0.tar.gz"
        // Text: sha256: ...

        // Let's try to match the filename in href and the sha256 in text.
        // Regex: `ruby-([0-9.]+).tar.gz.*?sha256:\s*([a-f0-9]{64})` (dot matches newline)
        // But regex crate doesn't support multiline dot match easily without flag s.

        let re = Regex::new(r"(?s)ruby-([0-9.]+)\.tar\.gz.*?sha256:\s+([a-f0-9]{64})").unwrap();
        // Wait, regex crate `.` does NOT match newline by default. `(?s)` enables it.

        let mut artifacts = Vec::new();

        for cap in re.captures_iter(&text) {
            let version = cap[1].to_string();
            let sha256 = cap[2].to_string();

            let parts: Vec<&str> = version.split('.').collect();
            if parts.len() < 2 {
                continue;
            }
            let minor = format!("{}.{}", parts[0], parts[1]);
            let url = format!("https://cache.ruby-lang.org/pub/ruby/{minor}/ruby-{version}.tar.gz");

            let artifact = Artifact {
                name: "ruby".to_string(),
                version: version.clone(),
                arch: "universal-apple-darwin".to_string(),
                url,
                sha256,
            };

            // Skip already-indexed versions
            if known_versions.contains(&version) {
                continue;
            }

            if artifact.validate().is_ok() {
                artifacts.push(artifact);
            }
        }

        // dedup by version
        artifacts.sort_by(|a, b| b.version.cmp(&a.version));
        artifacts.dedup_by(|a, b| a.version == b.version);

        println!("  Found {} Ruby artifacts", artifacts.len());
        Ok(artifacts)
    }
}

/// Strategy for discovering packages that must be built from source.
///
/// Uses GitHub releases to enumerate versions and produces `Artifact` records
/// with a placeholder `"BUILD_FROM_SOURCE"` SHA-256, signaling to the
/// installer that a source build is required rather than a binary download.
#[derive(Debug)]
pub struct BuildStrategy {
    name: String,
    source_url: String,
    tag_pattern: Option<String>,
    #[allow(dead_code)]
    spec: apl_schema::BuildSpec,
    client: Client,
}

impl BuildStrategy {
    /// Create a new `BuildStrategy` for a package with the given `name`,
    /// `source_url` (must be a GitHub URL), optional `tag_pattern` for
    /// extracting version strings from tags, and a `BuildSpec` describing
    /// how to compile the package.
    ///
    /// # Panics
    ///
    /// Panics if the `GITHUB_TOKEN` environment variable contains characters
    /// that cannot be represented as an HTTP header value.
    pub fn new(
        name: String,
        source_url: String,
        tag_pattern: Option<String>,
        spec: apl_schema::BuildSpec,
    ) -> Self {
        let mut headers = reqwest::header::HeaderMap::new();
        if let Ok(token) = std::env::var("GITHUB_TOKEN") {
            headers.insert(
                reqwest::header::AUTHORIZATION,
                reqwest::header::HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
            );
        }

        let client = Client::builder()
            .user_agent("apl-builder/0.1.0")
            .default_headers(headers)
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            name,
            source_url,
            tag_pattern,
            spec,
            client,
        }
    }
}

#[async_trait]
impl Strategy for BuildStrategy {
    async fn fetch_artifacts(
        &self,
        known_versions: &HashSet<String>,
        _cache: &mut StrategyCache,
    ) -> Result<Vec<Artifact>> {
        let tag_pattern = self.tag_pattern.as_deref().unwrap_or("{{version}}");
        let mut discovered_versions = Vec::new();

        // 1. Resolve versions
        if self.source_url.contains("github.com") {
            let parts: Vec<&str> = self.source_url.split('/').collect();
            if parts.len() < 5 {
                anyhow::bail!("Invalid GitHub source URL: {}", self.source_url);
            }
            let owner = parts[3];
            let repo = parts[4].trim_end_matches(".git");

            println!("    [Discovery] Fetching releases for {owner}/{repo}...");

            let releases =
                crate::indexer::forges::github::fetch_all_releases(&self.client, owner, repo)
                    .await?;

            for release in releases {
                if release.draft || release.prerelease {
                    continue;
                }

                // Extract version from tag using pattern
                let version_str = if tag_pattern == "{{version}}" {
                    crate::indexer::forges::github::strip_tag_prefix(&release.tag_name, &self.name)
                } else {
                    let prefix = tag_pattern.replace("{{version}}", "");
                    release.tag_name.trim_start_matches(&prefix).to_string()
                };

                if !known_versions.contains(&version_str) {
                    discovered_versions.push(version_str);
                }
            }
        } else {
            // Generic Git Discovery using ls-remote
            println!(
                "    [Discovery] Running git ls-remote on {}...",
                self.source_url
            );

            let output = Command::new("git")
                .arg("ls-remote")
                .arg("--tags")
                .arg(&self.source_url)
                .output()
                .await?;

            if !output.status.success() {
                anyhow::bail!(
                    "git ls-remote failed for {}: {}",
                    self.source_url,
                    String::from_utf8_lossy(&output.stderr)
                );
            }

            let stdout = String::from_utf8_lossy(&output.stdout);

            // Parse refs/tags/TAGNAME
            for line in stdout.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() < 2 {
                    continue;
                }
                let ref_name = parts[1];

                if let Some(tag) = ref_name.strip_prefix("refs/tags/") {
                    // Skip dereferenced tags (^{})
                    if tag.ends_with("^{}") {
                        continue;
                    }

                    // Extract version from tag using pattern
                    let version_str = if tag_pattern == "{{version}}" {
                        // Simple heuristic: try to strip package name if present
                        if tag.starts_with(&self.name) {
                            tag.trim_start_matches(&self.name)
                                .trim_start_matches(['-', '_', 'v'])
                                .to_string()
                        } else {
                            tag.trim_start_matches('v').to_string()
                        }
                    } else {
                        let prefix = tag_pattern.replace("{{version}}", "");
                        if !tag.starts_with(&prefix) {
                            continue;
                        }
                        tag.trim_start_matches(&prefix).to_string()
                    };

                    if !version_str.is_empty() && !known_versions.contains(&version_str) {
                        discovered_versions.push(version_str);
                    }
                }
            }
        }

        // 2. Apply version_pattern filter
        if let Some(pattern) = &self.spec.version_pattern {
            let regex_str = if pattern.contains('*') {
                pattern.replace('.', "\\.").replace('*', ".*")
            } else {
                pattern.clone()
            };

            if let Ok(re) = regex::Regex::new(&format!("^{regex_str}$")) {
                discovered_versions.retain(|v| re.is_match(v));
            }
        }

        // 3. Generate Artifacts
        let mut artifacts = Vec::new();
        for version in discovered_versions {
            let tag = tag_pattern.replace("{{version}}", &version);

            // Construct download URL
            let url = if self.source_url.contains("github.com") {
                // GitHub Archive URL
                let parts: Vec<&str> = self.source_url.split('/').collect();
                let owner = parts[3];
                let repo = parts[4].trim_end_matches(".git");
                format!("https://github.com/{owner}/{repo}/archive/refs/tags/{tag}.tar.gz")
            } else if let Some(template) = &self.spec.download_url_template {
                template.replace("{{version}}", &version)
            } else {
                // For generic git, we don't have a guaranteed tarball URL unless we know the forge.
                // But `apl-builder` or `installer` handles `git clone` if URL ends in .git?
                // Actually, looking at Artifact struct, it expects a URL.
                // If we return the git URL, the installer must know how to handle it.
                // Or we append `#tag` to `source_url`.
                format!("{}#{}", self.source_url, tag)
            };

            artifacts.push(Artifact {
                name: self.name.clone(),
                version,
                arch: "source".to_string(),
                url, // If not github, this is git_url#tag
                sha256: "BUILD_FROM_SOURCE".to_string(),
            });
        }

        println!(
            "    Found {} potential new versions for {}",
            artifacts.len(),
            self.name
        );
        Ok(artifacts)
    }
}
