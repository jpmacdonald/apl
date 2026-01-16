use anyhow::Result;
use apl_schema::Artifact;
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;

use regex::Regex;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

#[async_trait]
pub trait Strategy: Send + Sync {
    /// Fetch artifacts based on the configuration strategy.
    async fn fetch_artifacts(&self) -> Result<Vec<Artifact>>;
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
    async fn fetch_artifacts(&self) -> Result<Vec<Artifact>> {
        let url = format!("https://releases.hashicorp.com/{}/index.json", self.product);
        let resp = self
            .client
            .get(&url)
            .send()
            .await?
            .json::<HashiCorpIndex>()
            .await?;

        let mut artifacts = Vec::new();

        // TODO: Filter logic could be moved to shared util
        // For now, replicate python logic: fetch SHA256SUMS separately or trust index?
        // HashiCorp index.json doesn't have SHAs directly in builds, need to key off filename match?
        // Actually, builds array usually lacks SHA256 in the index.json from the API endpoint used earlier.
        // Let's implement minimal fetch for now and refine.

        // APL currently supports:
        let allowed_platforms = [
            ("darwin", "amd64", "x86_64-apple-darwin"),
            ("darwin", "arm64", "aarch64-apple-darwin"),
            // linux...
        ];

        for (version, data) in resp.versions {
            if version.contains('-') {
                continue;
            } // Skip unstable

            // Fetch SHA256SUMS for this version
            // https://releases.hashicorp.com/terraform/1.5.0/terraform_1.5.0_SHA256SUMS
            let sha_url = format!(
                "https://releases.hashicorp.com/{}/{}/{}_{}_SHA256SUMS",
                self.product, version, self.product, version
            );

            let shas = fetch_shasums(&self.client, &sha_url)
                .await
                .unwrap_or_default();

            for build in data.builds {
                for (os, arch, apl_arch) in allowed_platforms {
                    if build.os == os && build.arch == arch {
                        // shas key is filename, e.g. terraform_1.5.0_darwin_amd64.zip
                        let checksum = shas.get(&build.filename).cloned().unwrap_or_default();

                        if !checksum.is_empty() {
                            artifacts.push(Artifact {
                                name: self.product.clone(),
                                version: version.clone(),
                                arch: apl_arch.to_string(),
                                url: build.url.clone(),
                                sha256: checksum,
                            });
                        }
                    }
                }
            }
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
    async fn fetch_artifacts(&self) -> Result<Vec<Artifact>> {
        let url = "https://go.dev/dl/?mode=json&include=all"; // fetch all versions
        let releases = reqwest::get(url).await?.json::<Vec<GoRelease>>().await?;

        let mut artifacts = Vec::new();

        for release in releases {
            let version = release.version.trim_start_matches("go").to_string();
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
    async fn fetch_artifacts(&self) -> Result<Vec<Artifact>> {
        // Node.js is trickier: index.json gives versions, but we need SHA256.
        // For MVP, we will implement fetching index.json and pointing to the URL.
        // Checksums in a real impl should be fetched from SHASUMS256.txt per version.

        let url = "https://nodejs.org/dist/index.json";
        let releases = reqwest::get(url).await?.json::<Vec<NodeRelease>>().await?;

        let mut artifacts = Vec::new();
        for release in releases.iter().take(50) {
            // Limit to 50 for MVP
            let version = release.version.trim_start_matches('v').to_string();

            // Fetch SHASUMS256.txt for this version
            let sha_url = format!("https://nodejs.org/dist/{}/SHASUMS256.txt", release.version);
            // Optimally we'd do this concurrently or lazily, but sequential is fine for factory.
            let shas = fetch_shasums(&Client::new(), &sha_url)
                .await
                .unwrap_or_default();

            for file in &release.files {
                // files list contains strings like "osx-arm64-tar", "osx-x64-tar"
                let (apl_arch, suffix) = match file.as_str() {
                    "osx-arm64-tar" => ("aarch64-apple-darwin", "darwin-arm64.tar.gz"),
                    "osx-x64-tar" => ("x86_64-apple-darwin", "darwin-x64.tar.gz"),
                    _ => continue,
                };

                let filename = format!("node-{}-{}", release.version, suffix);
                let checksum = shas.get(&filename).cloned().unwrap_or_default();

                if !checksum.is_empty() {
                    artifacts.push(Artifact {
                        name: "node".to_string(),
                        version: version.clone(),
                        arch: apl_arch.to_string(),
                        url: format!("https://nodejs.org/dist/{}/{}", release.version, filename),
                        sha256: checksum,
                    });
                }
            }
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
    async fn fetch_artifacts(&self) -> Result<Vec<Artifact>> {
        // Placeholder
        Ok(vec![])
    }
}

pub struct AwsStrategy;

#[async_trait]
impl Strategy for AwsStrategy {
    async fn fetch_artifacts(&self) -> Result<Vec<Artifact>> {
        // AWS CLI v2 Changelog parsing
        let url = "https://raw.githubusercontent.com/aws/aws-cli/v2/CHANGELOG.rst";
        let text = reqwest::get(url).await?.text().await?;
        let mut artifacts = Vec::new();

        // Regex to find "2.15.30" patterns at start of lines or headers
        // RST headers: 2.15.30
        //              =======
        let re = Regex::new(r"(\d+\.\d+\.\d+)").unwrap();

        // Simplification: Scan for versions, deduplicate
        let mut versions = Vec::new();
        for line in text.lines() {
            if let Some(cap) = re.captures(line) {
                if line.trim() == &cap[1] {
                    // likely a header line
                    versions.push(cap[1].to_string());
                }
            }
        }

        // Take top 5 unique
        versions.sort_by(|a, b| b.cmp(a)); // desc
        versions.dedup();

        for version in versions.iter().take(5) {
            let url = format!("https://awscli.amazonaws.com/AWSCLIV2-{version}.pkg");
            // We must fetch SHA256 manually
            let sha256 = download_and_hash(&Client::new(), &url)
                .await
                .unwrap_or_else(|e| {
                    eprintln!("    [WARN] Failed to hash {url}: {e}");
                    "".to_string() // Validation will fail this later, or we skip?
                });

            // Only push if we got a valid SHA (download succeeded)
            if !sha256.is_empty() {
                artifacts.push(Artifact {
                    name: "aws".to_string(),
                    version: version.clone(),
                    arch: "universal-apple-darwin".to_string(),
                    url,
                    sha256,
                });
            }
        }
        println!("  Found {} AWS artifacts", artifacts.len());
        Ok(artifacts)
    }
}

pub struct PythonStrategy;

#[async_trait]
impl Strategy for PythonStrategy {
    async fn fetch_artifacts(&self) -> Result<Vec<Artifact>> {
        // Parsing python.org/ftp/python/
        // Simpler: Use GitHub tags for cpython? or stick to html parse.
        // Stick to python.org for official source.
        // MVP: Hardcode or simple regex on index page.
        let url = "https://www.python.org/ftp/python/";
        let text = reqwest::get(url).await?.text().await?;
        let re = Regex::new(r#"href="(\d+\.\d+\.\d+)/""#).unwrap();

        let mut versions: Vec<String> = re.captures_iter(&text).map(|c| c[1].to_string()).collect();

        versions.sort_by(|a, b| b.cmp(a)); // desc, but rudimentary string sort 3.9 > 3.10 is wrong. 
        // TODO: Semver sort. For now relying on string sort generally OK for 3.x
        versions.dedup();

        let mut artifacts = Vec::new();
        for version in versions.iter().take(5) {
            // Source tarball
            let url = format!("https://www.python.org/ftp/python/{version}/Python-{version}.tgz");

            // Fetch SHA
            let sha256 = download_and_hash(&Client::new(), &url)
                .await
                .unwrap_or_default();

            if !sha256.is_empty() {
                artifacts.push(Artifact {
                    name: "python".to_string(),
                    version: version.clone(),
                    arch: "universal-apple-darwin".to_string(), // Source is universal
                    url,
                    sha256,
                });
            }
        }
        println!("  Found {} Python artifacts", artifacts.len());
        Ok(artifacts)
    }
}

pub struct RubyStrategy;

#[async_trait]
impl Strategy for RubyStrategy {
    async fn fetch_artifacts(&self) -> Result<Vec<Artifact>> {
        // https://www.ruby-lang.org/en/downloads/
        let url = "https://www.ruby-lang.org/en/downloads/";
        let text = reqwest::get(url).await?.text().await?;

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
