pub mod discovery;
pub mod hashing;
pub mod walk;

pub use discovery::*;
pub use hashing::HashCache;
pub use walk::{registry_path, walk_registry_toml_files};

use crate::core::index::{HashType, IndexBinary, IndexSource, PackageIndex, VersionInfo};
use crate::package::{DiscoveryConfig, Package, PackageTemplate};
use anyhow::Result;
use reqwest::Client;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::registry::github::GithubRelease;

/// Generate index from algorithmic registry templates
pub async fn generate_index_from_registry(
    client: &Client,
    registry_dir: &Path,
    package_filter: Option<&str>,
) -> Result<PackageIndex> {
    let hash_cache = Arc::new(Mutex::new(HashCache::load()));
    let mut index = PackageIndex::new();

    let toml_files = walk_registry_toml_files(registry_dir)?;

    // Pass 1: Collect templates and identify GitHub repos to fetch
    let mut templates = Vec::new();
    let mut repos_to_fetch = Vec::new();
    // Map of (owner, repo) -> Vec<PackageName>
    // Multiple packages might share a repo (rare but possible), or we just need to know which package needs which repo
    let mut pkg_repo_map: HashMap<String, (String, String)> = HashMap::new();

    for template_path in toml_files {
        let toml_str = match fs::read_to_string(&template_path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("⚠ Failed to read {}: {}", template_path.display(), e);
                continue;
            }
        };

        let template: PackageTemplate = match toml::from_str(&toml_str) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("⚠ Failed to parse {}: {}", template_path.display(), e);
                continue;
            }
        };

        if let Some(filter) = package_filter {
            if template.package.name.to_string() != filter {
                continue;
            }
        }

        if let DiscoveryConfig::GitHub { github, .. } = &template.discovery {
            if let Ok(repo_ref) = crate::GitHubRepo::new(github) {
                let key = (repo_ref.owner().to_string(), repo_ref.name().to_string());
                if !repos_to_fetch.contains(&key) {
                    repos_to_fetch.push(key.clone());
                }
                pkg_repo_map.insert(template.package.name.to_string(), key);
            }
        }

        templates.push((template_path, template));
    }

    // Pass 2: Batch fetch metadata from GitHub via GraphQL
    // We only have a token in the client if the user provided one, but GraphQL requires it.
    // If no token, we might fail or fall back (but for now we assume token exists for indexer).
    let token = std::env::var("GITHUB_TOKEN").unwrap_or_default();
    let mut master_release_cache: HashMap<(String, String), Vec<GithubRelease>> = HashMap::new();

    if !repos_to_fetch.is_empty() {
        println!(
            "   Fetching metadata for {} repositories (in batches)...",
            repos_to_fetch.len()
        );

        for chunk in repos_to_fetch.chunks(10) {
            match crate::registry::graphql::fetch_batch_releases(client, &token, chunk).await {
                Ok(batch_results) => {
                    master_release_cache.extend(batch_results);
                }
                Err(e) => {
                    eprintln!("   ⚠ Batch fetch failed: {}", e);
                    // We continue, so individual packages will fail gracefully downstream if data is missing
                }
            }
        }
    }

    // Pass 3: Process each package using cached metadata
    for (template_path, template) in templates {
        println!("   Processing {}...", template_path.display());

        let pkg_name = template.package.name.to_string();

        // Discover versions using the master cache or manual config
        let (versions, releases_map) = match &template.discovery {
            DiscoveryConfig::GitHub {
                tag_pattern,
                semver_only,
                include_prereleases,
                ..
            } => {
                // Look up in master cache
                let releases = if let Some(key) = pkg_repo_map.get(&pkg_name) {
                    master_release_cache.get(key).cloned().unwrap_or_default()
                } else {
                    Vec::new()
                };

                // If empty and we are supposed to have data, warn?
                if releases.is_empty() {
                    // Could be a failed batch fetch or just a repo with no releases
                }

                let mut versions = Vec::new();
                let mut map = HashMap::new();

                for release in releases {
                    map.insert(release.tag_name.clone(), release.clone());

                    if !include_prereleases && release.prerelease {
                        continue;
                    }

                    let version = extract_version_from_tag(&release.tag_name, tag_pattern);

                    if *semver_only && semver::Version::parse(&version).is_err() {
                        continue;
                    }

                    versions.push(version);
                }

                (versions, Some(Arc::new(map)))
            }
            DiscoveryConfig::Manual { manual } => (manual.clone(), None),
        };

        // If versions is empty, verify if it was a fetch error or just zero versions
        if versions.is_empty() {
            eprintln!("     ⚠ No versions found");
            // Determine if we should count this as an error or just a skip
            continue;
        }

        println!("     Found {} versions", versions.len());

        // Hydrate each version in parallel
        use futures::stream::{self, StreamExt};

        println!(
            "     Hydrating {} versions (concurrently)...",
            versions.len()
        );

        let versions_stream = stream::iter(versions)
            .map(|version| {
                let client = client.clone();
                let template_clone = template.clone();
                let hash_cache_clone = hash_cache.clone();
                let releases_map_clone = releases_map.clone();
                let ver_str = version.clone();
                async move {
                    let res = package_to_index_ver(
                        &client,
                        &template_clone,
                        &version,
                        hash_cache_clone,
                        releases_map_clone,
                    )
                    .await;
                    (ver_str, res)
                }
            })
            .buffer_unordered(20); // Concurrency limit

        let results: Vec<(String, Result<VersionInfo>)> = versions_stream.collect().await;

        let mut skipped_versions = Vec::new();
        let total = results.len();
        let mut processed = 0;

        for (ver_str, res) in results {
            match res {
                Ok(ver_info) => {
                    index.upsert_release(
                        &template.package.name.to_string(),
                        &template.package.description,
                        if template.package.type_ == crate::package::PackageType::App {
                            "app"
                        } else {
                            "cli"
                        },
                        ver_info,
                    );
                    processed += 1;
                }
                Err(_e) => {
                    // Collect skipped versions for summary
                    skipped_versions.push(ver_str);
                }
            }
        }

        if processed == 0 && total > 0 {
            eprintln!("     ⚠ No valid versions found! Check TOML configuration.");
        }

        if !skipped_versions.is_empty() {
            // Sort versions to make output deterministic
            skipped_versions.sort_by(|a, b| {
                // Try to sort by semver if possible
                let sem_a = semver::Version::parse(a).ok();
                let sem_b = semver::Version::parse(b).ok();
                match (sem_a, sem_b) {
                    (Some(va), Some(vb)) => vb.cmp(&va), // Descending
                    _ => b.cmp(a),
                }
            });

            println!(
                "     (Skipped incompatible versions: {})",
                skipped_versions.join(", ")
            );
        }
    }

    hash_cache.lock().await.save()?;
    Ok(index)
}

/// Generate index from legacy flat packages directory

pub async fn generate_index_from_dir(
    client: &Client,
    packages_dir: &Path,
    package_filter: Option<&str>,
) -> Result<PackageIndex> {
    let hash_cache = Arc::new(Mutex::new(HashCache::load()));
    let mut index = PackageIndex::new();

    for entry in fs::read_dir(packages_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().is_some_and(|e| e == "toml") {
            let name = path.file_stem().unwrap().to_string_lossy();

            if let Some(filter) = package_filter {
                if name != filter {
                    continue;
                }
            }

            println!("   Processing {name}...");

            let toml_str = fs::read_to_string(&path)?;
            let pkg: Package = toml::from_str(&toml_str)?;

            // Convert to VersionInfo
            let mut binaries = Vec::new();
            for (arch, bin) in pkg.binary {
                let hash = get_or_compute_hash(client, &bin.url, hash_cache.clone()).await?;

                binaries.push(IndexBinary {
                    arch,
                    url: bin.url,
                    hash: crate::Sha256Hash::new(hash),
                    hash_type: HashType::Sha256,
                });
            }

            // Legacy Package always has source
            let source_hash =
                get_or_compute_hash(client, &pkg.source.url, hash_cache.clone()).await?;
            let source = Some(IndexSource {
                url: pkg.source.url,
                hash: crate::Sha256Hash::new(source_hash),
                hash_type: HashType::Sha256,
            });

            let ver_info = VersionInfo {
                version: pkg.package.version.to_string(),
                binaries,
                source,
                deps: pkg.dependencies.runtime,
                build_deps: pkg.dependencies.build,
                build_script: pkg.build.map(|b| b.script).unwrap_or_default(),
                bin: pkg.install.bin,
                hints: pkg.hints.post_install,
                app: None,
            };

            index.upsert_release(
                &pkg.package.name.to_string(),
                &pkg.package.description,
                "cli",
                ver_info,
            );
        }
    }

    hash_cache.lock().await.save()?;
    Ok(index)
}

pub async fn package_to_index_ver(
    client: &Client,
    template: &PackageTemplate,
    version: &str,
    hash_cache: Arc<Mutex<HashCache>>,
    releases_map: Option<Arc<HashMap<String, GithubRelease>>>,
) -> Result<VersionInfo> {
    let mut binaries = Vec::new();

    if let Some(ref targets) = template.assets.targets {
        for (target, arch_str) in targets {
            let url = template
                .assets
                .url_template
                .replace("{{version}}", version)
                .replace("{{target}}", arch_str);

            // Resolve hash
            let hash_res = resolve_hash(
                client,
                template,
                version,
                &url,
                hash_cache.clone(),
                releases_map.clone(),
            )
            .await;

            let hash = match hash_res {
                Ok(h) => h,
                Err(e) if e.to_string().contains("not found in GitHub release") => {
                    continue;
                }
                Err(e) => return Err(e),
            };

            // Parse target as Arch
            let arch: crate::Arch = target
                .parse()
                .map_err(|e| anyhow::anyhow!("Invalid architecture '{}': {}", target, e))?;

            binaries.push(IndexBinary {
                arch,
                url,
                hash: crate::Sha256Hash::new(hash),
                hash_type: HashType::Sha256,
            });
        }
    } else if template.assets.universal {
        let url = template.assets.url_template.replace("{{version}}", version);
        let hash_res = resolve_hash(
            client,
            template,
            version,
            &url,
            hash_cache.clone(),
            releases_map.clone(),
        )
        .await;

        match hash_res {
            Ok(hash) => {
                binaries.push(IndexBinary {
                    arch: crate::Arch::Universal,
                    url,
                    hash: crate::Sha256Hash::new(hash),
                    hash_type: HashType::Sha256,
                });
            }
            Err(e) if e.to_string().contains("not found in GitHub release") => {
                // Skip universal if not found
            }
            Err(e) => return Err(e),
        }
    }

    if binaries.is_empty() {
        anyhow::bail!("No supported binaries found for version {}", version);
    }

    Ok(VersionInfo {
        version: version.to_string(),
        binaries,
        source: None,
        deps: Vec::new(),
        build_deps: Vec::new(),
        build_script: String::new(),
        bin: template.install.bin.clone(),
        hints: template.hints.post_install.clone(),
        app: None,
    })
}

async fn resolve_hash(
    client: &Client,
    template: &PackageTemplate,
    version: &str,
    asset_url: &str,
    hash_cache: Arc<Mutex<HashCache>>,
    releases_map: Option<Arc<HashMap<String, GithubRelease>>>,
) -> Result<String> {
    {
        let cache = hash_cache.lock().await;
        if let Some((hash, _type)) = cache.get(asset_url) {
            return Ok(hash);
        }
    }

    if let DiscoveryConfig::GitHub { .. } = template.discovery {
        let filename = crate::filename_from_url(asset_url);
        let tag = template
            .discovery
            .tag_pattern()
            .replace("{{version}}", version);

        if let Some(map) = releases_map {
            if let Some(release) = map.get(&tag) {
                // Check if the asset actually exists in the release
                if !release.assets.iter().any(|a| a.name == filename) {
                    anyhow::bail!("Asset '{}' not found in GitHub release {}", filename, tag);
                }

                if let Ok(hash) = resolve_digest_from_github(client, release, filename).await {
                    hash_cache.lock().await.insert(
                        asset_url.to_string(),
                        hash.clone(),
                        HashType::Sha256,
                    );
                    return Ok(hash);
                }
            }
        }

        // Fallback or if map prevents lookup (though we should have the map)
        // Actually, if we have the map and didn't find the release, resolving blindly via API will also fail (404)
        // But the old logic called get_github_asset_digest which internally fetched releases.
        // We have refactored get_github_asset_digest to resolve_digest_from_github which takes a RELEASE.
        // So we CANNOT call the old function anymore.
        // If map is None (should not happen for GitHub), we might be stuck?
        // But we fetched releases logic above.

        // If we failed to find it in map, maybe we should error early?
        // For now, if we don't have a release, we can't digest.
        // Unless we keep the old function? I replaced it.
        // So we MUST find it in the map.
    }

    if let Some(ref checksum_url_template) = template.checksums.url_template {
        let checksum_url = checksum_url_template.replace("{{version}}", version);
        if let Ok(hash) = fetch_and_parse_checksum(client, &checksum_url, asset_url).await {
            hash_cache
                .lock()
                .await
                .insert(asset_url.to_string(), hash.clone(), HashType::Sha256);
            return Ok(hash);
        }
    }

    if template.checksums.skip {
        // Fallback: Download the asset to compute the hash
        let hash = compute_hash_from_url(client, asset_url).await?;
        hash_cache
            .lock()
            .await
            .insert(asset_url.to_string(), hash.clone(), HashType::Sha256);
        return Ok(hash);
    }

    anyhow::bail!(
        "Could not resolve checksum for {asset_url}. If this package does not provide a checksum, set [checksums] skip = true to allow downloading and computing it."
    )
}

/// Download an asset and compute its SHA256 hash
async fn compute_hash_from_url(client: &Client, url: &str) -> Result<String> {
    use futures::StreamExt;
    use sha2::Digest;

    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("Failed to download asset {}: {}", url, resp.status());
    }

    let mut hasher = sha2::Sha256::new();
    let mut stream = resp.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        hasher.update(&chunk);
    }

    let hash = format!("{:x}", hasher.finalize());
    Ok(hash)
}

async fn get_or_compute_hash(
    _client: &Client,
    url: &str,
    hash_cache: Arc<Mutex<HashCache>>,
) -> Result<String> {
    let cache = hash_cache.lock().await;
    if let Some((hash, _)) = cache.get(url) {
        return Ok(hash);
    }

    anyhow::bail!("Checksum not found in cache for {url}")
}

async fn fetch_and_parse_checksum(
    client: &Client,
    checksum_url: &str,
    asset_url: &str,
) -> Result<String> {
    let resp = client.get(checksum_url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("Checksum file not found: {checksum_url}");
    }

    let text = resp.text().await?;
    let filename = crate::filename_from_url(asset_url);
    if filename.is_empty() {
        anyhow::bail!("Invalid asset URL: {asset_url}");
    }

    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            let hash = parts[0];
            let file = parts[1].trim_start_matches('*');
            if file == filename || file.ends_with(filename) {
                return Ok(hash.to_string());
            }
        }
    }

    anyhow::bail!("Hash not found in checksum file for {filename}")
}

#[cfg(test)]
mod indexer_tests {
    use super::*;
    use crate::package::{AssetConfig, ChecksumConfig, DiscoveryConfig, InstallSpec};
    use crate::registry::github::{GithubAsset, GithubRelease};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[tokio::test]
    async fn test_asset_existence_logic() {
        let client = Client::new();
        let hash_cache = Arc::new(Mutex::new(HashCache::default()));

        let template = PackageTemplate {
            package: crate::package::PackageInfo {
                name: "test-pkg".into(),
                version: "1.0.0".into(),
                description: "test".to_string(),
                homepage: "".to_string(),
                license: "".to_string(),
                type_: crate::package::PackageType::Cli,
            },
            discovery: DiscoveryConfig::GitHub {
                github: "owner/repo".to_string(),
                tag_pattern: "v{{version}}".to_string(),
                semver_only: true,
                include_prereleases: false,
            },
            assets: AssetConfig {
                url_template: "https://example.com/v{{version}}/release-{{target}}.tar.gz"
                    .to_string(),
                targets: Some(
                    vec![("arm64".to_string(), "arm64".to_string())]
                        .into_iter()
                        .collect(),
                ),
                universal: false,
            },
            checksums: ChecksumConfig::default(),
            install: InstallSpec::default(),
            hints: crate::package::Hints::default(),
        };

        // Mock release map
        // 1.0.0 exists but MISSES the arm64 asset
        let mut map = HashMap::new();
        map.insert(
            "v1.0.0".to_string(),
            GithubRelease {
                id: 1,
                tag_name: "v1.0.0".to_string(),
                assets: vec![GithubAsset {
                    name: "release-x86_64.tar.gz".to_string(),
                    browser_download_url: "https://example.com/x86_64".to_string(),
                    digest: None,
                }],
                draft: false,
                prerelease: false,
                body: String::new(),
            },
        );

        let releases_map = Some(Arc::new(map));

        // Attempt to hydrate v1.0.0
        let result =
            package_to_index_ver(&client, &template, "1.0.0", hash_cache, releases_map).await;

        // SHOULD BAIL with "No supported binaries found" because the arm64 asset was skipped locally
        assert!(result.is_err());
        let err = result.err().unwrap().to_string();
        assert!(err.contains("No supported binaries found for version 1.0.0"));
        println!(
            "Test success: Version skipped gracefully because asset was missing from local map."
        );
    }
}
