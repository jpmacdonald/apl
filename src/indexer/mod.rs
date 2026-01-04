pub mod discovery;
pub mod hashing;
pub mod sources;
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

use sources::traits::{ListingSource, ReleaseInfo};

/// Generate index from algorithmic registry templates
pub async fn generate_index_from_registry(
    client: &Client,
    registry_dir: &Path,
    package_filter: Option<&str>,
) -> Result<PackageIndex> {
    let hash_cache = Arc::new(Mutex::new(HashCache::load()));
    let mut index = PackageIndex::new();

    let toml_files = walk_registry_toml_files(registry_dir)?;

    // Pass 1: Collect templates and identify sources to fetch
    let mut templates = Vec::new();
    let mut sources: Vec<Box<dyn ListingSource>> = Vec::new();
    // Map of package_name -> source_key
    let mut pkg_source_map: HashMap<String, String> = HashMap::new();

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
            if let Ok(repo_ref) = crate::types::GitHubRepo::new(github) {
                let source = sources::github::GitHubSource {
                    owner: repo_ref.owner().to_string(),
                    repo: repo_ref.name().to_string(),
                };
                let key = source.key();

                // Check if we already have this source
                if !pkg_source_map.values().any(|k| k == &key) {
                    sources.push(Box::new(source));
                }
                pkg_source_map.insert(template.package.name.to_string(), key);
            }
        }

        templates.push((template_path, template));
    }

    println!(
        "   🔍 Discovered {} unique sources in registry/",
        sources.len()
    );

    // Pass 2: Fetch metadata from sources (in parallel)
    let mut master_release_cache: HashMap<String, Vec<ReleaseInfo>> = HashMap::new();

    if !sources.is_empty() {
        println!("   Fetching Metadata...");

        use futures::stream::{self, StreamExt};

        let mut fetch_stream = stream::iter(sources)
            .map(|source| {
                let client = client.clone();
                async move {
                    let key = source.key();
                    let result = source.fetch_releases(&client).await;
                    (key, result)
                }
            })
            .buffer_unordered(50); // Concurrent source fetches

        while let Some((key, result)) = fetch_stream.next().await {
            match result {
                Ok(releases) => {
                    master_release_cache.insert(key, releases);
                }
                Err(e) => {
                    eprintln!("   ✗ {} ({})", key, e);
                }
            }
        }
    }

    // Pass 3: Process each package using cached metadata (in parallel)
    println!("   Processing Packages...");
    use futures::stream::{self, StreamExt};
    let pkg_source_map = Arc::new(pkg_source_map);
    let master_release_cache = Arc::new(master_release_cache);

    let mut results_stream = stream::iter(templates)
        .map(|(_template_path, template)| {
            let client = client.clone();
            let hash_cache = hash_cache.clone();
            let pkg_source_map_clone = pkg_source_map.clone();
            let master_release_cache_clone = master_release_cache.clone();

            async move {
                let pkg_name = template.package.name.to_string();

                // Discover versions using the master cache or manual config
                let (versions, releases_map): (
                    Vec<(String, String, String)>,
                    Option<Arc<HashMap<String, ReleaseInfo>>>,
                ) = match &template.discovery {
                    DiscoveryConfig::GitHub {
                        tag_pattern,
                        include_prereleases,
                        ..
                    } => {
                        let releases = if let Some(key) = pkg_source_map_clone.get(&pkg_name) {
                            master_release_cache_clone
                                .get(key)
                                .cloned()
                                .unwrap_or_default()
                        } else {
                            Vec::new()
                        };

                        let sorted_releases = releases;

                        let mut versions = Vec::new();
                        let mut map = HashMap::new();

                        for release in sorted_releases {
                            map.insert(release.tag_name.clone(), release.clone());

                            if !include_prereleases && release.prerelease {
                                continue;
                            }

                            let extracted =
                                extract_version_from_tag(&release.tag_name, tag_pattern);

                            if let Some(normalized) = auto_parse_version(&extracted) {
                                versions.push((release.tag_name.clone(), extracted, normalized));
                            }
                        }

                        (versions, Some(Arc::new(map)))
                    }
                    DiscoveryConfig::Manual { manual } => {
                        let tuples = manual
                            .iter()
                            .map(|v| (v.clone(), v.clone(), v.clone()))
                            .collect();
                        (tuples, None)
                    }
                };

                if versions.is_empty() {
                    return (
                        template.clone(),
                        Vec::new(),
                        vec![anyhow::anyhow!("no versions found")],
                    );
                }

                // Process versions in parallel (nested stream)
                let versions_stream = stream::iter(versions)
                    .map(|(full_tag, extracted, normalized)| {
                        let client = client.clone();
                        let template_clone = template.clone();
                        let hash_cache_clone = hash_cache.clone();
                        let releases_map_clone = releases_map.clone();
                        let display_ver = normalized.clone();
                        let url_ver = extracted.clone();
                        let tag_for_lookup = full_tag.clone();
                        async move {
                            let res = package_to_index_ver(
                                &client,
                                &template_clone,
                                &tag_for_lookup,
                                &url_ver,
                                &display_ver,
                                hash_cache_clone,
                                releases_map_clone,
                            )
                            .await;
                            (display_ver, res)
                        }
                    })
                    .buffer_unordered(10); // Concurrent versions per package

                let version_results: Vec<(String, Result<VersionInfo>)> =
                    versions_stream.collect().await;

                let mut v_infos = Vec::new();
                let mut errors = Vec::new();

                for (_ver, res) in version_results {
                    match res {
                        Ok(info) => v_infos.push(info),
                        Err(e) => errors.push(e),
                    }
                }

                (template, v_infos, errors)
            }
        })
        .buffer_unordered(20); // Concurrent packages

    let mut total_releases = 0;
    while let Some((template, v_infos, errors)) = results_stream.next().await {
        let pkg_name = template.package.name.to_string();
        let success_count = v_infos.len();
        let total_versions = v_infos.len() + errors.len();
        total_releases += success_count;

        for ver_info in v_infos {
            index.upsert_release(
                &pkg_name,
                &template.package.description,
                if template.package.type_ == crate::package::PackageType::App {
                    "app"
                } else {
                    "cli"
                },
                ver_info,
            );
        }

        // Align package name at 20 characters
        let name_col = format!("{:<20}", pkg_name);

        if !errors.is_empty() {
            let error_msg = errors[0].to_string();
            let human_err = humanize_error(&error_msg);

            if success_count > 0 {
                println!(
                    "   ⚠ {} {}/{} versions",
                    name_col, success_count, total_versions
                );
                println!("     └ {}", human_err);
            } else {
                eprintln!("   ✗ {} 0/{} versions", name_col, total_versions);
                eprintln!("     └ {}", human_err);
            }
        } else {
            println!("   ✓ {} {} versions indexed", name_col, total_versions);
        }
    }

    hash_cache.lock().await.save()?;

    let index_file = registry_dir.join("../index.bin");
    let size_bytes = fs::metadata(&index_file).map(|m| m.len()).unwrap_or(0);
    println!();
    println!(
        "   Done: {}KB index.bin ({} total releases)",
        size_bytes / 1024,
        total_releases
    );
    Ok(index)
}

fn humanize_error(e: &str) -> String {
    if e.contains("No supported binaries found") {
        "No supported binaries for this platform (macOS) in most releases".to_string()
    } else if e.contains("Could not resolve checksum") {
        "Checksum missing in GitHub release (set [checksums] skip = true to allow)".to_string()
    } else if e.contains("no versions found") {
        "No versions found for this repository".to_string()
    } else if e.contains("error decoding response body") {
        "Network error or GitHub API rate limit during metadata fetch".to_string()
    } else if e.contains("Asset") && e.contains("not found in GitHub release") {
        "Expected binary asset not found in GitHub release".to_string()
    } else {
        e.split('.').next().unwrap_or(e).to_string()
    }
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
            for (arch, bin) in pkg.targets {
                let hash = get_or_compute_hash(client, &bin.url, hash_cache.clone()).await?;

                binaries.push(IndexBinary {
                    arch,
                    url: bin.url,
                    hash: crate::types::Sha256Hash::new(hash),
                    hash_type: HashType::Sha256,
                });
            }

            // Legacy Package always has source
            let source_hash =
                get_or_compute_hash(client, &pkg.source.url, hash_cache.clone()).await?;
            let source = Some(IndexSource {
                url: pkg.source.url,
                hash: crate::types::Sha256Hash::new(source_hash),
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
                if pkg.package.type_ == crate::package::PackageType::App {
                    "app"
                } else {
                    "cli"
                },
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
    full_tag: &str, // Full GitHub tag for release map lookup (e.g., "gping-v1.20.1")
    url_version: &str, // Extracted version for URL templates (e.g., "1.20.1")
    display_version: &str, // Normalized version for display (e.g., "1.20.1")
    hash_cache: Arc<Mutex<HashCache>>,
    releases_map: Option<Arc<HashMap<String, ReleaseInfo>>>,
) -> Result<VersionInfo> {
    let mut binaries = Vec::new();

    if let Some(ref targets) = template.assets.targets {
        for (target, arch_str) in targets {
            let url = template
                .assets
                .url_template
                .replace("{{version}}", url_version)
                .replace("{{target}}", arch_str);

            // Resolve hash
            let hash_res = resolve_hash(
                client,
                template,
                full_tag,
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
                Err(e) => {
                    // Error captured in aggregated summary - no need to print per-version
                    return Err(e);
                }
            };

            // Parse target as Arch
            let arch: crate::types::Arch = target
                .parse()
                .map_err(|e| anyhow::anyhow!("Invalid architecture '{}': {}", target, e))?;

            binaries.push(IndexBinary {
                arch,
                url,
                hash: crate::types::Sha256Hash::new(hash),
                hash_type: HashType::Sha256,
            });
        }
    } else if template.assets.universal {
        let url = template
            .assets
            .url_template
            .replace("{{version}}", url_version);
        let hash_res = resolve_hash(
            client,
            template,
            full_tag,
            &url,
            hash_cache.clone(),
            releases_map.clone(),
        )
        .await;

        match hash_res {
            Ok(hash) => {
                binaries.push(IndexBinary {
                    arch: crate::types::Arch::Universal,
                    url,
                    hash: crate::types::Sha256Hash::new(hash),
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
        anyhow::bail!(
            "No supported binaries found for version {}",
            display_version
        );
    }

    Ok(VersionInfo {
        version: display_version.to_string(),
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
    releases_map: Option<Arc<HashMap<String, ReleaseInfo>>>,
) -> Result<String> {
    {
        let cache = hash_cache.lock().await;
        if let Some((hash, _type)) = cache.get(asset_url) {
            return Ok(hash);
        }
    }

    if let DiscoveryConfig::GitHub { .. } = template.discovery {
        let filename = crate::filename_from_url(asset_url);
        let tag = version;

        if let Some(map) = releases_map {
            if let Some(release) = map.get(version) {
                // Check if the asset actually exists in the release
                if !release.assets.iter().any(|a| a.name == filename) {
                    anyhow::bail!("Asset '{}' not found in GitHub release {}", filename, tag);
                }

                if let Ok(hash) = resolve_digest(client, release, filename).await {
                    hash_cache.lock().await.insert(
                        asset_url.to_string(),
                        hash.as_str().to_string(),
                        HashType::Sha256,
                    );
                    return Ok(hash.as_str().to_string());
                }
            }
        }
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

    if let Some(hash) = crate::indexer::discovery::scan_text_for_hash(&text, filename) {
        println!(
            "       ✓ Found hash for {} in {}",
            filename,
            crate::filename_from_url(checksum_url)
        );
        return Ok(hash);
    }

    anyhow::bail!("Hash not found in checksum file for {filename}")
}

#[cfg(test)]
mod indexer_tests {
    use super::*;
    use crate::package::{AssetConfig, ChecksumConfig, DiscoveryConfig, InstallSpec};
    use sources::traits::{AssetInfo, ReleaseInfo};
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
            ReleaseInfo {
                tag_name: "v1.0.0".to_string(),
                prerelease: false,
                body: String::new(),
                prune: false,
                assets: vec![AssetInfo {
                    name: "release-x86_64.tar.gz".to_string(),
                    download_url: "https://example.com/x86_64".to_string(),
                    digest: None,
                }],
            },
        );

        let releases_map = Some(Arc::new(map));

        // Attempt to hydrate v1.0.0
        let result = package_to_index_ver(
            &client,
            &template,
            "v1.0.0", // full_tag for map lookup
            "1.0.0",  // url_version for templates
            "1.0.0",  // display_version
            hash_cache,
            releases_map,
        )
        .await;

        // SHOULD BAIL with "No supported binaries found" because the arm64 asset was skipped locally
        assert!(result.is_err());
        let err = result.err().unwrap().to_string();
        assert!(err.contains("No supported binaries found for version 1.0.0"));
        println!(
            "Test success: Version skipped gracefully because asset was missing from local map."
        );
    }
}
