pub mod discovery;
pub mod hashing;
pub mod sources;
pub mod walk;

pub use discovery::*;
pub use hashing::HashCache;
pub use walk::{registry_path, walk_registry_toml_files};

use crate::core::index::{HashType, IndexBinary, PackageIndex, VersionInfo};
use crate::package::{DiscoveryConfig, PackageTemplate};
use anyhow::Result;
use reqwest::Client;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

use sources::traits::{ListingSource, ReleaseInfo};

/// Generate index from algorithmic registry templates
///
/// If `force_full` is false, attempts to load the existing index and only
/// deep-fetches packages whose latest version has changed (Optimistic Delta Hydration).
pub async fn generate_index_from_registry(
    client: &Client,
    registry_dir: &Path,
    package_filter: Option<&str>,
    force_full: bool,
) -> Result<PackageIndex> {
    let hash_cache = Arc::new(Mutex::new(HashCache::load()));

    // Phase 0: Load existing index (if not forcing full rebuild)
    let index_path = registry_dir.join("../index.bin");
    let mut index = if !force_full && index_path.exists() {
        match PackageIndex::load(&index_path) {
            Ok(existing) => {
                println!(
                    "   📦 Loaded existing index ({} packages)",
                    existing.packages.len()
                );
                existing
            }
            Err(e) => {
                eprintln!("   ⚠ Failed to load existing index: {e}. Rebuilding from scratch.");
                PackageIndex::new()
            }
        }
    } else {
        if force_full {
            println!("   🔄 Force full rebuild requested.");
        }
        PackageIndex::new()
    };

    let toml_files = walk_registry_toml_files(registry_dir)?;

    // Pass 1: Collect templates and identify sources to fetch
    let mut templates = Vec::new();
    let other_sources: Vec<Box<dyn ListingSource>> = Vec::new();
    let mut github_repos: Vec<crate::types::RepoKey> = Vec::new();

    // Map package_name -> RepoKey (for dirty checking)
    let mut pkg_repo_map: HashMap<String, crate::types::RepoKey> = HashMap::new();
    // Map package_name -> tag_pattern (for version extraction)
    let mut pkg_tag_pattern_map: HashMap<String, String> = HashMap::new();
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
            if template.package.name != filter {
                continue;
            }
        }

        match &template.discovery {
            DiscoveryConfig::GitHub {
                github,
                tag_pattern,
                ..
            } => {
                if let Ok(repo_ref) = crate::types::GitHubRepo::new(github) {
                    let key = crate::types::RepoKey {
                        owner: repo_ref.owner().to_string(),
                        repo: repo_ref.name().to_string(),
                    };
                    let source_key = format!("github:{}/{}", key.owner, key.repo);

                    if !pkg_source_map.values().any(|k| k == &source_key) {
                        github_repos.push(key.clone());
                    }
                    pkg_repo_map.insert(template.package.name.to_string(), key);
                    pkg_tag_pattern_map
                        .insert(template.package.name.to_string(), tag_pattern.clone());
                    pkg_source_map.insert(template.package.name.to_string(), source_key);
                }
            }
            // Add other source types here in the future
            _ => {}
        }

        templates.push((template_path, template));
    }

    println!(
        "   🔍 Discovered {} unique GitHub sources in registry/",
        github_repos.len()
    );

    // Pass 2: Delta Check (Optimistic) - fetch only latest tags
    let mut dirty_repos: Vec<crate::types::RepoKey> = Vec::new();
    let mut skipped_count = 0;

    if !force_full && !github_repos.is_empty() && !index.packages.is_empty() {
        println!("   ⚡ Checking for updates (lightweight)...");
        let token = std::env::var("GITHUB_TOKEN").unwrap_or_default();

        // Can batch ~20 repos safely for lightweight query
        for chunk in github_repos.chunks(20) {
            match sources::github::graphql::fetch_latest_versions_batch(client, &token, chunk).await
            {
                Ok(latest_versions) => {
                    for (key, remote_tag_opt) in latest_versions {
                        // Find package name for this repo
                        let pkg_name = pkg_repo_map
                            .iter()
                            .find(|(_, v)| **v == key)
                            .map(|(k, _)| k.as_str());

                        if let Some(name) = pkg_name {
                            let local_latest = index
                                .find(name)
                                .and_then(|e| e.latest())
                                .map(|v| v.version.as_str());

                            // Extract version from remote tag using package's tag_pattern (if available)
                            // IMPORTANT: Apply the same normalization as during indexing
                            let remote_version = remote_tag_opt.as_ref().and_then(|tag| {
                                let extracted = if let Some(pattern) = pkg_tag_pattern_map.get(name)
                                {
                                    discovery::extract_version_from_tag(tag, pattern)
                                } else {
                                    // Fallback: strip common prefixes
                                    tag.trim_start_matches('v').to_string()
                                };
                                // Normalize the same way indexing does
                                discovery::auto_parse_version(&extracted)
                            });

                            if local_latest.map(|s| s.to_string()) == remote_version {
                                skipped_count += 1;
                            } else {
                                dirty_repos.push(key);
                            }
                        } else {
                            // New package (not in pkg_repo_map)
                            dirty_repos.push(key);
                        }
                    }
                }
                Err(e) => {
                    eprintln!(
                        "   ⚠ Delta check failed: {e}. Falling back to full fetch for this batch."
                    );
                    dirty_repos.extend(chunk.iter().cloned());
                }
            }
        }

        if skipped_count > 0 {
            println!(
                "   ✓ {} packages unchanged, {} need update",
                skipped_count,
                dirty_repos.len()
            );
        }
    } else {
        // Force full or no existing index: all repos are "dirty"
        dirty_repos = github_repos.clone();
    }

    // Pass 3: Fetch metadata from dirty sources (in parallel / batched)
    let mut master_release_cache: HashMap<String, Vec<ReleaseInfo>> = HashMap::new();

    if !dirty_repos.is_empty() {
        println!(
            "   Fetching Metadata for {} packages (Batched GraphQL)...",
            dirty_repos.len()
        );
        let token = std::env::var("GITHUB_TOKEN").unwrap_or_default();

        // Chunk into groups of 4 (GraphQL complexity limit protection)
        for chunk in dirty_repos.chunks(4) {
            match sources::github::graphql::fetch_batch_releases(client, &token, chunk).await {
                Ok(batch_results) => {
                    for (key, releases) in batch_results {
                        // Convert GithubRelease to generic ReleaseInfo
                        let generic_releases: Vec<ReleaseInfo> = releases
                            .into_iter()
                            .map(|r| ReleaseInfo {
                                tag_name: r.tag_name,
                                prune: r.draft || r.prerelease,
                                body: r.body.unwrap_or_default(),
                                prerelease: r.prerelease,
                                assets: r
                                    .assets
                                    .into_iter()
                                    .map(|a| sources::traits::AssetInfo {
                                        name: a.name,
                                        download_url: a.browser_download_url,
                                        digest: a
                                            .digest
                                            .and_then(|d| crate::types::Sha256Digest::new(d).ok()),
                                    })
                                    .collect(),
                            })
                            .collect();

                        let source_key = format!("github:{}/{}", key.owner, key.repo);
                        master_release_cache.insert(source_key, generic_releases);
                    }
                }
                Err(e) => {
                    eprintln!("   ✗ Batch fetch failed: {e}");
                    // Fallback to individual fetches? Or just fail?
                    // For now, fail hard on the batch to warn user.
                }
            }
        }
    }

    // Process non-GitHub sources if any (future proofing)
    if !other_sources.is_empty() {
        use futures::stream::{self, StreamExt};
        let mut fetch_stream = stream::iter(other_sources)
            .map(|source| {
                let client = client.clone();
                async move {
                    let key = source.key();
                    let result = source.fetch_releases(&client).await;
                    (key, result)
                }
            })
            .buffer_unordered(15);

        while let Some((key, result)) = fetch_stream.next().await {
            match result {
                Ok(releases) => {
                    master_release_cache.insert(key, releases);
                }
                Err(e) => {
                    eprintln!("   ✗ {key} ({e})");
                }
            }
        }
    }

    // Build a set of dirty source keys for efficient lookup
    let dirty_source_keys: std::collections::HashSet<String> = dirty_repos
        .iter()
        .map(|key| format!("github:{}/{}", key.owner, key.repo))
        .collect();

    // Filter templates to only include dirty packages (unchanged packages are already in index)
    let templates_to_process: Vec<_> = if !force_full && !dirty_source_keys.is_empty() {
        templates
            .into_iter()
            .filter(|(_, template)| {
                pkg_source_map
                    .get(&template.package.name.to_string())
                    .is_some_and(|key| dirty_source_keys.contains(key))
            })
            .collect()
    } else {
        templates
    };

    // Pass 4: Process each dirty package using cached metadata (in parallel)
    println!("   Processing Packages...");
    use futures::stream::{self, StreamExt};
    let pkg_source_map = Arc::new(pkg_source_map);
    let master_release_cache = Arc::new(master_release_cache);

    let mut results_stream = stream::iter(templates_to_process)
        .map(|(_template_path, template)| {
            let client = client.clone();
            let hash_cache = hash_cache.clone();
            let pkg_source_map_clone = pkg_source_map.clone();
            let master_release_cache_clone = master_release_cache.clone();

            async move {
                let pkg_name = template.package.name.to_string();

                // Discover versions using the master cache or manual config
                #[allow(clippy::type_complexity)]
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
                                discovery::extract_version_from_tag(&release.tag_name, tag_pattern);

                            if let Some(normalized) = discovery::auto_parse_version(&extracted) {
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
                        let tag_for_lookup = full_tag.clone();
                        async move {
                            let res = package_to_index_ver(
                                &client,
                                &template_clone,
                                &tag_for_lookup,
                                &extracted,
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
    let mut fully_indexed = 0;
    let mut partial = 0;
    let mut failed = 0;

    while let Some((template, v_infos, errors)) = results_stream.next().await {
        let pkg_name = template.package.name.to_string();
        let success_count = v_infos.len();
        let total_versions = v_infos.len() + errors.len();
        total_releases += success_count;

        for ver_info in v_infos {
            // Infer type: App if .app is present, else Cli
            let kind = if ver_info.app.is_some() { "app" } else { "cli" };

            index.upsert_release(&pkg_name, &template.package.description, kind, ver_info);
        }

        // Align package name at 20 characters
        let name_col = format!("{pkg_name:<20}");
        if !errors.is_empty() {
            let error_msg = errors[0].to_string();
            let human_err = humanize_error(&error_msg);

            if success_count > 0 {
                partial += 1;
                println!("   ⚠ {name_col} {success_count}/{total_versions} versions");
                println!("     └ {human_err}");
            } else {
                failed += 1;
                eprintln!("   ✗ {name_col} 0/{total_versions} versions");
                eprintln!("     └ {human_err}");
            }
        } else {
            fully_indexed += 1;
            println!("   ✓ {name_col} {total_versions} versions indexed");
        }
    }

    hash_cache.lock().await.save()?;

    let index_file = registry_dir.join("../index.bin");
    let size_bytes = fs::metadata(&index_file).map(|m| m.len()).unwrap_or(0);
    let total_packages = fully_indexed + partial + failed;

    println!();
    println!(
        "   Done: {}KB index.bin ({total_releases} total releases)",
        size_bytes / 1024
    );
    println!();
    println!(
        "   {total_packages} packages: {fully_indexed} fully indexed, {partial} partial, {failed} failed"
    );

    Ok(index)
}

fn humanize_error(e: &str) -> String {
    if e.contains("No supported binaries found") {
        "Skipped: missing macOS binary assets".to_string()
    } else if e.contains("Could not resolve checksum") {
        "Skipped: checksums not available (set skip_checksums = true)".to_string()
    } else if e.contains("no versions found") {
        "No releases found in repository".to_string()
    } else if e.contains("error decoding response body") {
        "Skipped: network error or rate limit".to_string()
    } else if e.contains("Asset") && e.contains("not found in GitHub release") {
        "Skipped: expected asset not found".to_string()
    } else {
        format!("Skipped: {}", e.split('.').next().unwrap_or(e))
    }
}

pub async fn package_to_index_ver(
    client: &Client,
    template: &PackageTemplate,
    full_tag: &str, // Full GitHub tag for release map lookup (e.g., "gping-v1.20.1")
    _url_version: &str, // Extracted version for URL templates (e.g., "1.20.1")
    display_version: &str, // Normalized version for display (e.g., "1.20.1")
    hash_cache: Arc<Mutex<HashCache>>,
    releases_map: Option<Arc<HashMap<String, ReleaseInfo>>>,
) -> Result<VersionInfo> {
    let mut binaries = Vec::new();

    let release_info = releases_map
        .as_ref()
        .and_then(|map| map.get(full_tag))
        .ok_or_else(|| anyhow::anyhow!("Release {full_tag} not found in map"))?;

    // Strategy 1: Explicit selectors for each arch
    for (arch_name, selector) in &template.assets.select {
        if let Some(asset) = discovery::find_asset_by_selector(&release_info.assets, selector) {
            // Resolve hash
            let hash_res = resolve_hash(
                client,
                template,
                &asset.download_url,
                full_tag,
                hash_cache.clone(),
                releases_map.clone(),
            )
            .await;

            let hash = match hash_res {
                Ok(h) => h,
                Err(e) => {
                    // Fail the whole version if we can't get a hash for a matched asset
                    return Err(e);
                }
            };

            let arch: crate::types::Arch = arch_name.parse().map_err(|e| {
                anyhow::anyhow!("Invalid architecture identifier '{arch_name}': {e}")
            })?;

            binaries.push(IndexBinary {
                arch,
                url: asset.download_url.clone(),
                hash: crate::types::Sha256Hash::new(hash),
                hash_type: HashType::Sha256,
            });
        }
    }

    // Strategy 2: Universal binary
    if template.assets.universal {
        // For universal, we need a way to find it.
        // We'll use the "universal-macos" key or fallback to a heuristic if missing.
        let selector = template.assets.select.get("universal-macos").or(None);

        if let Some(selector) = selector {
            if let Some(asset) = discovery::find_asset_by_selector(&release_info.assets, selector) {
                let hash = resolve_hash(
                    client,
                    template,
                    &asset.download_url,
                    full_tag,
                    hash_cache.clone(),
                    releases_map.clone(),
                )
                .await?;

                binaries.push(IndexBinary {
                    arch: crate::types::Arch::Universal,
                    url: asset.download_url.clone(),
                    hash: crate::types::Sha256Hash::new(hash),
                    hash_type: HashType::Sha256,
                });
            }
        }
    }

    if binaries.is_empty() {
        anyhow::bail!("No supported binaries found for version {display_version}");
    }

    // Inference for 'bin'
    let bin_list = if let Some(ref b) = template.install.bin {
        b.clone()
    } else {
        // Default to package name
        vec![template.package.name.to_string()]
    };

    Ok(VersionInfo {
        version: display_version.to_string(),
        binaries,
        source: None,
        deps: Vec::new(),
        build_deps: Vec::new(),
        build_script: String::new(),
        bin: bin_list,
        hints: template.hints.post_install.clone(),
        app: template.install.app.clone(),
    })
}

async fn resolve_hash(
    client: &Client,
    template: &PackageTemplate,
    asset_url: &str,
    version: &str,
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
        let _tag = version; // Renamed to _tag as it's not directly used after this point

        if let Some(map) = releases_map {
            if let Some(release) = map.get(version) {
                // 1. Try resolving from release (assets or body)
                if let Ok(hash) = discovery::resolve_digest(client, release, filename).await {
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

    // 2. Try explicit checksum URL
    if let Some(ref checksum_url_template) = template.assets.checksum_url {
        let checksum_url = checksum_url_template.replace("{{version}}", version);
        if let Ok(hash) = fetch_and_parse_checksum(client, &checksum_url, asset_url).await {
            hash_cache
                .lock()
                .await
                .insert(asset_url.to_string(), hash.clone(), HashType::Sha256);
            return Ok(hash);
        }
    }

    // 3. Fallback: Download and compute if allowed
    if template.assets.skip_checksums {
        let hash = compute_hash_from_url(client, asset_url).await?;
        hash_cache
            .lock()
            .await
            .insert(asset_url.to_string(), hash.clone(), HashType::Sha256);
        return Ok(hash);
    }

    anyhow::bail!(
        "Could not resolve checksum for {asset_url}. If this package does not provide a checksum, set [assets] skip_checksums = true to allow downloading and computing it."
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
        return Ok(hash);
    }

    anyhow::bail!("Hash not found in checksum file for {filename}")
}

#[cfg(test)]
mod indexer_tests {
    use super::*;
    use crate::package::{AssetConfig, DiscoveryConfig, InstallSpec};
    use sources::traits::{AssetInfo, ReleaseInfo};
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    #[tokio::test]
    async fn test_asset_existence_logic() {
        let client = Client::new();
        let hash_cache = Arc::new(Mutex::new(HashCache::default()));

        let template = PackageTemplate {
            package: crate::package::PackageInfoTemplate {
                name: "test-pkg".into(),
                description: "test".to_string(),
                homepage: "".to_string(),
                license: "".to_string(),
            },
            discovery: DiscoveryConfig::GitHub {
                github: "owner/repo".to_string(),
                tag_pattern: "v{{version}}".to_string(),
                include_prereleases: false,
            },
            assets: AssetConfig {
                universal: false,
                select: {
                    let mut map = HashMap::new();
                    map.insert(
                        "arm64-macos".to_string(),
                        crate::package::AssetSelector::Suffix {
                            suffix: "arm64.tar.gz".to_string(),
                        },
                    );
                    map
                },
                skip_checksums: false,
                checksum_url: None,
            },
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
