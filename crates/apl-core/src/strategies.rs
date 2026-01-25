use anyhow::Result;
use apl_schema::Artifact;
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use futures::stream::{self, StreamExt};
use regex::Regex;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

#[derive(Serialize, Deserialize, Default, Clone, Debug)]
pub struct CacheEntry {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
}

pub type StrategyCache = HashMap<String, CacheEntry>;

#[async_trait]
pub trait Strategy: Send + Sync {
    /// Fetch artifacts based on the configuration strategy.
    ///
    /// `known_versions` contains versions already in the index - strategies should
    /// skip these to avoid redundant work. This enables incremental indexing.
    ///
    /// `cache` contains ETag/Last-Modified data for upstream URLs.
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
        .and_then(|h| h.to_str().ok().map(|s| s.to_string()));
    let last_modified = resp
        .headers()
        .get("last-modified")
        .and_then(|h| h.to_str().ok().map(|s| s.to_string()));

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

pub struct HashiCorpStrategy {
    product: String,
    client: Client,
}

impl HashiCorpStrategy {
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
            .and_then(|h| h.to_str().ok().map(|s| s.to_string()))
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

pub struct GolangStrategy;
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

pub struct GitHubStrategy {
    #[allow(dead_code)]
    owner: String,
    #[allow(dead_code)]
    repo: String,
}

impl GitHubStrategy {
    pub fn new(owner: String, repo: String) -> Self {
        Self { owner, repo }
    }
}

// ... GitHub Implementation ...

#[async_trait]
impl Strategy for GitHubStrategy {
    async fn fetch_artifacts(
        &self,
        _known_versions: &HashSet<String>,
        _cache: &mut StrategyCache,
    ) -> Result<Vec<Artifact>> {
        // Placeholder
        Ok(vec![])
    }
}

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

        let text = match fetch_text_with_cache(&Client::new(), url, cache).await? {
            Some(t) => t,
            None => {
                println!("  [Cache] AWS Changelog not modified");
                return Ok(vec![]);
            }
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
                        "".to_string()
                    });

                if !sha.is_empty() {
                    Some(Artifact {
                        name: "aws".to_string(),
                        version,
                        arch: "universal-apple-darwin".to_string(),
                        url,
                        sha256: sha,
                    })
                } else {
                    None
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

pub struct PythonStrategy;

#[async_trait]
impl Strategy for PythonStrategy {
    async fn fetch_artifacts(
        &self,
        known_versions: &HashSet<String>,
        cache: &mut StrategyCache,
    ) -> Result<Vec<Artifact>> {
        let url = "https://www.python.org/ftp/python/";
        let text = match fetch_text_with_cache(&Client::new(), url, cache).await? {
            Some(t) => t,
            None => {
                println!("  [Cache] Python index not modified");
                return Ok(vec![]);
            }
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

                if !sha.is_empty() {
                    Some(Artifact {
                        name: "python".to_string(),
                        version,
                        arch: "universal-apple-darwin".to_string(),
                        url,
                        sha256: sha,
                    })
                } else {
                    None
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
        let text = match fetch_text_with_cache(&Client::new(), url, cache).await? {
            Some(t) => t,
            None => {
                println!("  [Cache] Ruby index not modified");
                return Ok(vec![]);
            }
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

pub struct BuildStrategy {
    name: String,
    source_url: String,
    tag_pattern: Option<String>,
    #[allow(dead_code)]
    spec: apl_schema::BuildSpec,
    client: Client,
}

impl BuildStrategy {
    pub fn new(
        name: String,
        source_url: String,
        tag_pattern: Option<String>,
        spec: apl_schema::BuildSpec,
    ) -> Self {
        let client = Client::builder()
            .user_agent("apl-builder/0.1.0")
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
        // 1. Resolve versions from Source URL
        if !self.source_url.contains("github.com") {
            anyhow::bail!("BuildStrategy currently only supports GitHub source URLs for discovery");
        }

        let parts: Vec<&str> = self.source_url.split('/').collect();
        if parts.len() < 5 {
            anyhow::bail!("Invalid GitHub source URL: {}", self.source_url);
        }
        let owner = parts[3];
        let repo = parts[4].trim_end_matches(".git");

        println!("    [Discovery] Fetching releases for {owner}/{repo}...");

        let releases =
            crate::indexer::forges::github::fetch_all_releases(&self.client, owner, repo).await?;

        let tag_pattern = self.tag_pattern.as_deref().unwrap_or("{{version}}");

        let mut discovered_versions = Vec::new();
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

        // 2. Apply version_pattern filter
        if let Some(pattern) = &self.spec.version_pattern {
            let regex_str = if pattern.contains('*') {
                pattern.replace(".", "\\.").replace("*", ".*")
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
            let url = format!("https://github.com/{owner}/{repo}/archive/refs/tags/{tag}.tar.gz");

            artifacts.push(Artifact {
                name: self.name.clone(),
                version,
                arch: "source".to_string(),
                url,
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
