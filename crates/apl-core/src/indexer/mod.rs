/// Version discovery and asset resolution logic.
pub mod discovery;
/// Forge adapters for code hosting platforms.
pub mod forges;
/// Persistent hash caching for indexed artifacts.
pub mod hashing;
/// Importers for translating external package registries to APL TOML.
pub mod import;
/// Registry directory traversal utilities.
pub mod walk;

pub use discovery::*;
pub use hashing::HashCache;
pub use walk::{registry_path, walk_registry_toml_files};

use crate::package::{DiscoveryConfig, PackageTemplate};
use crate::types::{Arch, PackageName, RepoKey, Sha256Hash};
use anyhow::Result;
use apl_schema::Sha256Digest;
use apl_schema::index::{HashType, IndexBinary, PackageIndex, VersionInfo};
use reqwest::Client;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

use forges::traits::{ListingSource, ReleaseInfo};

use crate::io::artifacts::{ArtifactStore, get_artifact_store};

/// Generate index from algorithmic registry templates
///
/// If `force_full` is false, attempts to load the existing index and only
/// deep-fetches packages whose latest version has changed (Optimistic Delta Hydration).
/// # Errors
/// Returns an error if registry IO fails, network requests fail, or if manifest parsing fails.
pub async fn generate_index_from_registry(
    _client: &Client,
    registry_dir: &Path,
    package_filter: Option<&str>,
    force_full: bool,
    _verbose: bool,
    _reporter: Arc<dyn crate::Reporter>,
) -> Result<PackageIndex> {
    use futures::stream;
    use futures::stream::{self as fstream, StreamExt as FStreamExt};

    // Configure client with timeout (overshadowing the argument)
    let client = reqwest::Client::builder()
        .user_agent("apl/0.1.0")
        .timeout(std::time::Duration::from_secs(300))
        .build()?;

    let hash_cache = Arc::new(Mutex::new(HashCache::load()));

    // Initialize artifact store (optional, only if configured)
    let artifact_store: Option<Arc<ArtifactStore>> = get_artifact_store().await;

    if artifact_store.is_some() {
        println!("  artifact store enabled (mirroring to r2)");
    }

    // Phase 0: Load existing index (if not forcing full rebuild)
    let index_path = registry_dir.join("../index");
    let mut index = if !force_full && index_path.exists() {
        match PackageIndex::load(&index_path) {
            Ok(existing) => {
                println!(
                    "  loaded existing index ({} packages)",
                    existing.packages.len()
                );
                existing
            }
            Err(e) => {
                eprintln!("  failed to load existing index: {e}, rebuilding from scratch");
                PackageIndex::new()
            }
        }
    } else {
        if force_full {
            println!("  force full rebuild requested");
        }
        PackageIndex::new()
    };

    // Set mirror_base_url if artifact store is configured
    if let Some(ref store) = artifact_store {
        // Get public base URL from config (strip /cas/ suffix if present)
        let base = store.public_url("").trim_end_matches("/cas/").to_string();
        index.mirror_base_url = Some(base);
    }

    // Print header
    println!();
    println!("  regenerating index");
    if force_full {
        println!("  force full rebuild");
    }
    println!();

    println!("  discovering sources");

    // Phase 1: Discovery
    let toml_files: Vec<_> = walk_registry_toml_files(registry_dir)?.collect();
    let _total_files = toml_files.len() as u64;

    // Pass 1: Collect templates
    // Track all packages found in the registry for pruning stale entries
    let mut valid_packages = std::collections::HashSet::new();

    let mut templates = Vec::new();
    let other_sources: Vec<Box<dyn ListingSource>> = Vec::new();
    let _other_sources = other_sources;
    let mut github_repos: Vec<RepoKey> = Vec::new();
    let mut ports_repos: Vec<String> = Vec::new(); // package names to Look up in ports

    // Map package_name -> RepoKey (for dirty checking)
    let mut pkg_repo_map: HashMap<String, RepoKey> = HashMap::new();
    let mut pkg_tag_pattern_map: HashMap<String, String> = HashMap::new();
    let mut pkg_source_map: HashMap<String, String> = HashMap::new();

    for template_path in toml_files {
        let Ok(toml_str) = fs::read_to_string(&template_path) else {
            continue; // Squelch error for clean UI
        };

        let template: PackageTemplate = match toml::from_str(&toml_str) {
            Ok(t) => t,
            Err(_) => continue,
        };

        if let Some(filter) = package_filter {
            if template.package.name != filter {
                continue;
            }
        }

        if let DiscoveryConfig::GitHub {
            github,
            tag_pattern,
            ..
        } = &template.discovery
        {
            if let Ok(repo_ref) = crate::types::GitHubRepo::new(github) {
                let key = RepoKey {
                    owner: repo_ref.owner().to_string(),
                    repo: repo_ref.name().to_string(),
                };
                let source_key = format!("github:{}/{}", key.owner, key.repo);

                if !pkg_source_map.values().any(|k| k == &source_key) {
                    github_repos.push(key.clone());
                }
                pkg_repo_map.insert(template.package.name.to_string(), key);
                pkg_tag_pattern_map.insert(template.package.name.to_string(), tag_pattern.clone());
                pkg_source_map.insert(template.package.name.to_string(), source_key);
            }
        } else if let DiscoveryConfig::Ports { name } = &template.discovery {
            let source_key = format!("ports:{name}");
            if !pkg_source_map.values().any(|k| k == &source_key) {
                ports_repos.push(name.clone());
            }
            pkg_tag_pattern_map
                .insert(template.package.name.to_string(), "{{version}}".to_string());
            pkg_source_map.insert(template.package.name.to_string(), source_key);
        }

        valid_packages.insert(template.package.name.to_string());
        templates.push((template_path, template));
    }
    println!(
        "  {} github sources, {} ports",
        github_repos.len(),
        ports_repos.len()
    );

    // Pass 2: Delta Check
    // We can use a spinner here if fast, or skip visual if super fast.
    // The request didn't show "checking deltas".
    // It showed "discovering sources" then "fetching metadata".
    // Delta check is kind of part of discovery/fetching.
    // Let's merge it conceptually or just perform it quickly.

    let mut dirty_repos: Vec<RepoKey> = Vec::new();
    let mut _skipped_count = 0;

    if !force_full && !github_repos.is_empty() && !index.packages.is_empty() {
        // Optional: Add a quick spinner for delta check if needed
        // let pb_delta = multi.add(ProgressBar::new_spinner());
        // pb_delta.set_style(style.clone());
        // pb_delta.set_label("checking updates");

        let token = std::env::var("GITHUB_TOKEN").unwrap_or_default();

        // Batch fetch latest versions
        for chunk in github_repos.chunks(20) {
            match forges::github::graphql::fetch_latest_versions_batch(&client, &token, chunk).await
            {
                Ok(latest_versions) => {
                    for (key, remote_tag_opt) in latest_versions {
                        // Logic same as before
                        let pkg_name = pkg_repo_map
                            .iter()
                            .find(|(_, v)| **v == key)
                            .map(|(k, _)| k.as_str());

                        if let Some(name) = pkg_name {
                            let local_latest = index
                                .find(name)
                                .and_then(|e| e.latest())
                                .map(|v| v.version.as_str());

                            let remote_version = remote_tag_opt.as_ref().and_then(|tag| {
                                let extracted = if let Some(pattern) = pkg_tag_pattern_map.get(name)
                                {
                                    discovery::extract_version_from_tag(tag, pattern)
                                } else {
                                    tag.trim_start_matches('v').to_string()
                                };
                                discovery::auto_parse_version(&extracted)
                            });

                            if local_latest.map(std::string::ToString::to_string) == remote_version
                            {
                                _skipped_count += 1;
                            } else {
                                dirty_repos.push(key);
                            }
                        } else {
                            dirty_repos.push(key);
                        }
                    }
                }
                Err(_) => {
                    dirty_repos.extend(chunk.iter().cloned());
                }
            }
        }
        // pb_delta.finish_and_clear();
    } else {
        dirty_repos.clone_from(&github_repos);
    }

    // Metadata fetching (parallelized)
    let mut master_release_cache: HashMap<String, Vec<ReleaseInfo>> = HashMap::new();
    let total_dirty = dirty_repos.len();
    println!("  fetching metadata for {total_dirty} repositories");

    if !dirty_repos.is_empty() {
        let token = std::env::var("GITHUB_TOKEN").unwrap_or_default();
        let token = std::sync::Arc::new(token);

        // Create chunks and process them concurrently
        let chunks: Vec<Vec<RepoKey>> = dirty_repos.chunks(4).map(<[RepoKey]>::to_vec).collect();

        let results: Vec<
            Result<HashMap<RepoKey, Vec<forges::github::GithubRelease>>, anyhow::Error>,
        > = fstream::iter(chunks)
            .map(|chunk| {
                let client = client.clone();
                let token = token.clone();
                async move {
                    let result =
                        forges::github::graphql::fetch_batch_releases(&client, &token, &chunk)
                            .await;
                    let repos_str: Vec<_> = chunk
                        .iter()
                        .map(|k| format!("{}/{}", k.owner, k.repo))
                        .collect();
                    println!("    {}", repos_str.join(", "));
                    result
                }
            })
            .buffer_unordered(12) // Run up to 12 batches concurrently
            .collect()
            .await;

        // Process results
        let mut fetch_errors = Vec::new();
        for result in results {
            match result {
                Ok(batch_results) => {
                    for (key, releases) in batch_results {
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
                                    .map(|a| forges::traits::AssetInfo {
                                        name: a.name,
                                        download_url: a.browser_download_url,
                                        digest: a.digest.and_then(|d| Sha256Digest::new(d).ok()),
                                    })
                                    .collect(),
                            })
                            .collect();

                        let source_key = format!("github:{}/{}", key.owner, key.repo);
                        master_release_cache.insert(source_key, generic_releases);
                    }
                }
                Err(e) => {
                    fetch_errors.push(e.to_string());
                }
            }
        }

        if !fetch_errors.is_empty() {
            println!(
                "   {len} batch(es) failed: {errs}",
                len = fetch_errors.len(),
                errs = fetch_errors.join("; ")
            );
        }
    }
    println!("  {} repositories updated", master_release_cache.len());

    // Fetch ports metadata
    if !ports_repos.is_empty() {
        let bucket_url =
            std::env::var("APL_R2_BUCKET_URL").unwrap_or_else(|_| "https://apl.pub".to_string());
        println!(
            "  fetching metadata for {} ports from {bucket_url}",
            ports_repos.len()
        );

        for port_name in &ports_repos {
            match forges::ports::fetch_releases(&client, port_name, &bucket_url).await {
                Ok(releases) => {
                    let source_key = format!("ports:{port_name}");
                    master_release_cache.insert(source_key, releases);
                }
                Err(e) => {
                    println!("    failed to fetch port {port_name}: {e}");
                }
            }
        }
    }

    // Build dirty keys set
    let mut dirty_source_keys: std::collections::HashSet<String> = dirty_repos
        .iter()
        .map(|key| format!("github:{}/{}", key.owner, key.repo))
        .collect();
    // Ports are always considered "dirty"/fast-check for now since we don't have etags yet
    for port_name in &ports_repos {
        dirty_source_keys.insert(format!("ports:{port_name}"));
    }

    // Pass 3.5: Build Graph
    let mut stub_index = PackageIndex::new();
    for (_, template) in &templates {
        stub_index.upsert_release(
            &template.package.name,
            &template.package.description,
            "cli",
            template.package.tags.clone(),
            VersionInfo {
                version: "0.0.0".into(),
                binaries: vec![],
                deps: template.dependencies.runtime.clone(),
                build_deps: template.build.as_ref().map_or_else(
                    || template.dependencies.build.clone(),
                    |b| b.dependencies.clone(),
                ),
                bin: vec![],
                hints: String::new(),
                app: None,
                source: None,
                build_script: String::new(),
            },
        );
    }
    let layers = crate::resolver::resolve_build_plan(&stub_index)?;
    let template_map: HashMap<PackageName, (std::path::PathBuf, PackageTemplate)> = templates
        .iter()
        .map(|(p, t)| (t.package.name.clone(), (p.clone(), t.clone())))
        .collect();

    println!("  processing packages");

    let pkg_source_map = Arc::new(pkg_source_map);
    let master_release_cache = Arc::new(master_release_cache);

    let mut _total_releases = 0;
    let mut fully_indexed = 0;
    let mut partial = 0;
    let mut failed = 0;

    for layer in layers {
        let mut layer_templates = Vec::new();
        // Filter dirty
        for pkg_name in layer {
            if let Some((_path, template)) = template_map.get(pkg_name.as_str()) {
                let is_dirty = force_full
                    || pkg_source_map
                        .get(pkg_name.as_str())
                        .is_some_and(|key| dirty_source_keys.contains(key));

                if is_dirty {
                    layer_templates.push(template.clone());
                }
            }
        }

        if layer_templates.is_empty() {
            continue;
        }

        let index_snapshot = Arc::new(index.clone());

        let results_stream = stream::iter(layer_templates)
            .map(|template| {
                let client = client.clone();
                let hash_cache = hash_cache.clone();
                let pkg_source_map_clone = pkg_source_map.clone();
                let master_release_cache_clone = master_release_cache.clone();
                let index_ref = index_snapshot.clone();

                async move {
                    let pkg_name = template.package.name.to_string();

                    // (Discovery reuse logic)
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

                            let mut versions = Vec::new();
                            let mut map = HashMap::new();

                            for release in releases {
                                map.insert(release.tag_name.clone(), release.clone());
                                if !include_prereleases && release.prerelease {
                                    continue;
                                }
                                let extracted = discovery::extract_version_from_tag(
                                    &release.tag_name,
                                    tag_pattern,
                                );
                                if let Some(normalized) = discovery::auto_parse_version(&extracted)
                                {
                                    versions.push((release.tag_name, extracted, normalized));
                                }
                            }
                            (versions, Some(Arc::new(map)))
                        }
                        DiscoveryConfig::Ports { name: _ } => {
                            let releases = if let Some(key) = pkg_source_map_clone.get(&pkg_name) {
                                master_release_cache_clone
                                    .get(key)
                                    .cloned()
                                    .unwrap_or_default()
                            } else {
                                Vec::new()
                            };

                            let mut versions = Vec::new();
                            let mut map = HashMap::new();
                            for release in releases {
                                map.insert(release.tag_name.clone(), release.clone());
                                // Ports implicitly use semver/simple tags
                                versions.push((
                                    release.tag_name.clone(),
                                    release.tag_name.clone(),
                                    release.tag_name.clone(),
                                ));
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
                            template,
                            Vec::new(),
                            vec![anyhow::anyhow!("no versions found")],
                        );
                    }

                    // Limit to last 5 versions for performance
                    let versions: Vec<_> = versions.into_iter().take(5).collect();

                    let versions_stream = stream::iter(versions)
                        .map(|(full_tag, extracted, normalized)| {
                            let client_ref = client.clone();
                            let tmpl_ref = template.clone();
                            let hc_ref = hash_cache.clone();
                            let rm_ref = releases_map.clone();
                            let dv = normalized.clone();
                            let tf = full_tag.clone();
                            let ir = index_ref.clone();
                            async move {
                                let ctx = IndexingContext {
                                    client: &client_ref,
                                    hash_cache: hc_ref,
                                    releases_map: rm_ref,
                                    index: &ir,
                                };
                                let res =
                                    package_to_index_ver(ctx, &tmpl_ref, &tf, &extracted, &dv)
                                        .await;
                                (dv, res)
                            }
                        })
                        .buffer_unordered(8);

                    let version_results: Vec<(String, Result<VersionInfo>)> =
                        versions_stream.collect().await;
                    let mut v_infos = Vec::new();
                    let mut errors = Vec::new();
                    for (_, res) in version_results {
                        match res {
                            Ok(i) => v_infos.push(i),
                            Err(e) => errors.push(e),
                        }
                    }
                    (template, v_infos, errors)
                }
            })
            .buffer_unordered(16);

        let mut layer_results_stream = results_stream;
        let mut processed_count = 0;
        while let Some((template, v_infos, errors)) = layer_results_stream.next().await {
            let pkg_name = template.package.name.to_string();

            // Periodic save of hash cache
            processed_count += 1;
            if processed_count % 10 == 0 {
                let _ = hash_cache.lock().await.save();
            }

            let success_count = v_infos.len();
            let total_versions = v_infos.len() + errors.len();
            _total_releases += success_count;

            for ver_info in v_infos {
                let kind = if ver_info.app.is_some() { "app" } else { "cli" };
                index.upsert_release(
                    &pkg_name,
                    &template.package.description,
                    kind,
                    template.package.tags.clone(),
                    ver_info,
                );
            }

            // Ensure the package exists in the index even if no versions were successfully indexed.
            // This is critical for dependency resolution of library packages that might not
            // have binary assets yet.
            if success_count == 0 {
                index.upsert(apl_schema::index::IndexEntry {
                    name: pkg_name.clone(),
                    description: template.package.description.to_string(),
                    homepage: template.package.homepage.clone(),
                    type_: "cli".to_string(),
                    bins: vec![],
                    releases: vec![],
                    tags: template.package.tags.clone(),
                });
            }

            // Print per-package result: success is silent, problems surface with reason
            if errors.is_empty() {
                println!("    {pkg_name}");
            } else if success_count > 0 {
                println!("    {pkg_name:<25} {success_count}/{total_versions} partial");
            } else {
                let reason = humanize_error(&errors[0].to_string());
                println!("    {pkg_name:<25} skipped: {reason}");
            }

            if errors.is_empty() {
                fully_indexed += 1;
            } else if success_count > 0 {
                partial += 1;
            } else {
                failed += 1;
            }
        }
    }

    // Phase 4: Pruning stale packages
    // Only prune if we didn't filter by a specific package (which would prune everything else)
    if package_filter.is_none() {
        let initial_count = index.packages.len();
        index.packages.retain(|p| valid_packages.contains(&p.name));
        let pruned = initial_count - index.packages.len();
        if pruned > 0 {
            println!("    pruned {pruned} stale packages");
        }
    }

    // Set index timestamp (UTC)
    index.updated_at = chrono::Utc::now().timestamp();

    hash_cache.lock().await.save()?;

    let total_packages = fully_indexed + partial + failed;
    println!();
    println!("  index complete, {total_packages} packages");

    Ok(index)
}

fn humanize_error(e: &str) -> String {
    if e.contains("No supported binaries found") {
        "missing macOS binary assets".to_string()
    } else if e.contains("Could not resolve checksum") {
        "checksums not available".to_string()
    } else if e.contains("no versions found") {
        "no releases found".to_string()
    } else if e.contains("error decoding response body") {
        "network error or rate limit".to_string()
    } else if e.contains("Asset") && e.contains("not found in GitHub release") {
        "expected asset not found".to_string()
    } else {
        e.split('.').next().unwrap_or(e).to_lowercase()
    }
}

/// Shared context passed to per-version indexing tasks.
///
/// Holds a reference to the HTTP client, a shared hash cache, an optional
/// pre-fetched release map, and the current package index snapshot.
#[derive(Debug)]
pub struct IndexingContext<'a> {
    /// HTTP client used for downloading assets and checksums.
    pub client: &'a Client,
    /// Thread-safe, persistent hash cache to avoid redundant downloads.
    pub hash_cache: Arc<Mutex<HashCache>>,
    /// Pre-fetched release metadata keyed by tag name, if available.
    pub releases_map: Option<Arc<HashMap<String, ReleaseInfo>>>,
    /// Snapshot of the current package index for reuse checks.
    pub index: &'a PackageIndex,
}

/// Convert a single package template and version tag into a [`VersionInfo`] entry.
///
/// Reuses cached binary data when the version is already present in the index,
/// otherwise resolves assets, computes hashes, and optionally hydrates from
/// source using the build specification.
///
/// # Errors
///
/// Returns an error if asset resolution, hash computation, or source hydration
/// fails for the given version.
pub async fn package_to_index_ver(
    ctx: IndexingContext<'_>,
    template: &PackageTemplate,
    full_tag: &str,
    _url_version: &str,
    display_version: &str,
    // hash_cache: Arc<Mutex<HashCache>>,
    // releases_map: Option<Arc<HashMap<String, ReleaseInfo>>>,
    // index: &PackageIndex,
) -> Result<VersionInfo> {
    // Phase 1 Optimization: Check if this version is already indexed
    // If we have existing binaries for this version, reuse them to avoid re-hashing or re-building.
    // We still re-generate the metadata (deps, bin, etc.) from the current template to ensure
    // definition updates propagate to old versions without a full rebuild.
    if let Some(pkg_entry) = ctx.index.find(&template.package.name) {
        if let Some(existing_ver) = pkg_entry
            .releases
            .iter()
            .find(|v| v.version == display_version)
        {
            if !existing_ver.binaries.is_empty() {
                // tracing::debug!("      Reusing cached binaries for {}", display_version);
                return Ok(VersionInfo {
                    version: display_version.to_string(),
                    binaries: existing_ver.binaries.clone(),
                    source: None,
                    deps: template.dependencies.runtime.clone(),
                    build_deps: template.build.as_ref().map_or_else(
                        || template.dependencies.build.clone(),
                        |b| b.dependencies.clone(),
                    ),
                    build_script: template
                        .build
                        .as_ref()
                        .map_or_else(String::new, |b| b.script.clone()),
                    bin: template
                        .install
                        .bin
                        .clone()
                        .unwrap_or_else(|| vec![template.package.name.to_string()]),
                    hints: template.hints.post_install.clone(),
                    app: template.install.app.clone(),
                });
            }
        }
    }

    // Strategy 0: Build from source (Registry Hydration)
    if let Some(build_spec) = &template.build {
        let store = get_artifact_store().await.ok_or_else(|| {
            anyhow::anyhow!(
                "Package {display_version} has [build] spec but Artifact Store is disabled/unconfigured. Cannot hydrate."
            )
        })?;

        match hydrate_from_source(
            ctx.client,
            template,
            full_tag,
            display_version,
            build_spec,
            &store,
            ctx.index,
            Arch::current(),
        )
        .await
        {
            Ok(info) => return Ok(info),
            Err(e) => {
                anyhow::bail!("Build-from-Source failed for {display_version}: {e}");
            }
        }
    }

    let release_info = if let Some(map) = ctx.releases_map.as_ref() {
        map.get(full_tag)
            .ok_or_else(|| anyhow::anyhow!("Release {full_tag} not found in map"))?
            .clone()
    } else {
        // For Manual discovery, we don't have a map, so we create a stub
        ReleaseInfo {
            tag_name: full_tag.to_string(),
            prune: false,
            body: String::new(),
            prerelease: false,
            assets: vec![forges::traits::AssetInfo {
                name: full_tag.to_string(), // Heuristic for manual
                download_url: String::new(),
                digest: None,
            }],
        }
    };

    let mut binaries = Vec::new();

    // Asset Selection
    // Default to Auto selection for standard macOS architectures if no explicit selectors are provided.
    let selectors: Vec<(String, crate::package::AssetSelector)> =
        if template.assets.select.is_empty() {
            vec![
                (
                    "arm64-macos".to_string(),
                    crate::package::AssetSelector::Auto { auto: true },
                ),
                (
                    "x86_64-macos".to_string(),
                    crate::package::AssetSelector::Auto { auto: true },
                ),
            ]
        } else {
            template.assets.select.clone().into_iter().collect()
        };

    for (arch_name, selector) in &selectors {
        if let Some(asset) =
            discovery::find_asset_by_selector(&release_info.assets, selector, arch_name)
        {
            // Use pre-computed digest if available (e.g., ports from R2)
            // Otherwise resolve hash from checksum files or download
            let hash = if let Some(digest) = &asset.digest {
                digest.as_str().to_string()
            } else {
                let hash_res = resolve_hash(
                    ctx.client,
                    template,
                    &asset.download_url,
                    full_tag,
                    ctx.hash_cache.clone(),
                    ctx.releases_map.clone(),
                )
                .await;

                match hash_res {
                    Ok(h) => h,
                    Err(e) => {
                        // Fail the whole version if we can't get a hash for a matched asset
                        return Err(e);
                    }
                }
            };

            let arch: crate::types::Arch = arch_name.parse().map_err(|e| {
                anyhow::anyhow!("Invalid architecture identifier '{arch_name}': {e}")
            })?;

            binaries.push(IndexBinary {
                arch,
                url: asset.download_url.clone(),
                hash: crate::types::Sha256Hash::new(hash.clone()),
                hash_type: HashType::Sha256,
            });

            // Mirror asset to CAS if store is enabled
            // Skip mirroring for URLs already on apl.pub (ports are already in R2)
            if let Some(store) = get_artifact_store().await {
                if !asset.download_url.contains("apl.pub") {
                    if let Err(e) =
                        mirror_asset(ctx.client, &asset.download_url, &hash, &store).await
                    {
                        tracing::warn!("      Failed to mirror {}: {}", asset.name, e);
                    }
                }
            }
        }
    }

    if binaries.is_empty() {
        tracing::warn!(
            "      No supported binaries found for version {display_version}. Metadata will still be indexed."
        );
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
        deps: template.dependencies.runtime.clone(),
        build_deps: template.build.as_ref().map_or_else(
            || template.dependencies.build.clone(),
            |b| b.dependencies.clone(),
        ),
        build_script: template
            .build
            .as_ref()
            .map_or_else(String::new, |b| b.script.clone()),
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

/// Download an asset, compute its SHA256 hash, and save it to disk
async fn download_and_hash(client: &Client, url: &str, dest: &Path) -> Result<String> {
    use futures::StreamExt;
    use sha2::Digest;
    use tokio::io::AsyncWriteExt;

    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("Failed to download asset {}: {}", url, resp.status());
    }

    let mut file = tokio::fs::File::create(dest).await?;
    let mut hasher = sha2::Sha256::new();
    let mut stream = resp.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        file.write_all(&chunk).await?;
        hasher.update(&chunk);
    }

    file.flush().await?;
    let hash = format!("{:x}", hasher.finalize());
    Ok(hash)
}

/// Download an asset and compute its SHA256 hash without saving to disk
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

/// Build a package from source, bundle the output, and upload it to the artifact store.
///
/// Downloads the source archive, extracts it, resolves build dependencies from
/// the index, runs the build script inside a [`Sysroot`](crate::sysroot::Sysroot),
/// and uploads the resulting `tar.zst` bundle to the configured
/// [`ArtifactStore`].
///
/// # Errors
///
/// Returns an error if the source download, extraction, dependency resolution,
/// build execution, or artifact upload fails.
#[allow(clippy::too_many_arguments)]
pub async fn hydrate_from_source(
    client: &Client,
    template: &PackageTemplate,
    full_tag: &str,
    display_version: &str,
    build_spec: &crate::package::BuildSpec,
    store: &ArtifactStore,
    index: &PackageIndex,
    target_arch: Arch,
) -> Result<VersionInfo> {
    use crate::builder::Builder;
    use crate::sysroot::Sysroot;
    use sha2::Digest;

    // 1. Resolve Source URL
    let source_url = match &template.source {
        Some(s) => {
            let mut url = s
                .url
                .replace("{{tag}}", full_tag)
                .replace("{{version}}", display_version);

            if let Ok(v) = semver::Version::parse(display_version) {
                url = url
                    .replace("{{version_major}}", &v.major.to_string())
                    .replace("{{version_minor}}", &v.minor.to_string())
                    .replace("{{version_patch}}", &v.patch.to_string());
            } else {
                // Fallback for non-strict semver: simple split
                let parts: Vec<&str> = display_version.split('.').collect();
                if !parts.is_empty() {
                    url = url.replace("{{version_major}}", parts[0]);
                }
                if parts.len() >= 2 {
                    url = url.replace("{{version_minor}}", parts[1]);
                }
                if parts.len() >= 3 {
                    url = url.replace("{{version_patch}}", parts[2]);
                }
            }
            url
        }
        None => {
            // Heuristic for GitHub
            if let DiscoveryConfig::GitHub { github, .. } = &template.discovery {
                format!("https://github.com/{github}/archive/refs/tags/{full_tag}.tar.gz")
            } else {
                anyhow::bail!("No source URL provided for build-from-source template");
            }
        }
    };

    tracing::debug!("      Hydrating from source: {source_url}");

    // 2. Prepare Directories
    let tmp_dir = tempfile::tempdir()?;
    let source_archive = tmp_dir.path().join("source.tar.gz");
    let extract_dir = tmp_dir.path().join("src");
    let build_dir = tmp_dir.path().join("build");

    std::fs::create_dir_all(&extract_dir)?;
    std::fs::create_dir_all(&build_dir)?;

    // 3. Download Source and compute hash
    download_and_hash(client, &source_url, &source_archive).await?;

    // 4. Extract
    crate::io::extract::extract_auto(
        &source_archive,
        &extract_dir,
        &crate::reporter::NullReporter,
        &crate::types::PackageName::from("source"),
        &crate::types::Version::from("0.0.0"),
        None,
    )?;
    crate::io::extract::strip_components(&extract_dir)?;

    // 5. Resolve and download dependencies
    let mut build_deps = Vec::new();
    let mut dep_tmps = Vec::new();

    for dep_name in &build_spec.dependencies {
        // Find dependency in the index
        if let Some(entry) = index.find(dep_name) {
            if let Some(latest) = entry.latest() {
                if let Some(bin) = latest
                    .binaries
                    .iter()
                    .find(|b| b.arch == target_arch || b.arch == Arch::Universal)
                {
                    tracing::debug!(
                        "      Satisfying build dep: {} ({})",
                        dep_name,
                        latest.version
                    );

                    let dep_tmp = tempfile::tempdir()?;
                    let dep_archive = dep_tmp.path().join("dep.archive");

                    // Download the dependency artifact
                    let resp = client.get(&bin.url).send().await?;
                    if !resp.status().is_success() {
                        anyhow::bail!(
                            "Failed to download dependency {}: {}",
                            dep_name,
                            resp.status()
                        );
                    }
                    let content = resp.bytes().await?;
                    std::fs::write(&dep_archive, content)?;

                    // Extract it to a dedicated directory for mounting
                    let dep_extract_dir = tmp_dir.path().join("deps").join(dep_name);
                    crate::io::extract::extract_auto(
                        &dep_archive,
                        &dep_extract_dir,
                        &crate::reporter::NullReporter,
                        &crate::types::PackageName::from(dep_name.as_str()),
                        &crate::types::Version::from(latest.version.as_str()),
                        None,
                    )?;

                    build_deps.push((dep_name.clone(), dep_extract_dir));
                    dep_tmps.push(dep_tmp);
                }
            }
        }
    }

    // 6. Build in Sysroot
    let sysroot = Sysroot::new()?;
    let builder = Builder::new(&sysroot);
    let log_path = crate::build_log_path(
        &format!("{}-{}", template.package.name, target_arch),
        display_version,
    );

    builder.build(
        &extract_dir,
        &build_deps,
        &build_spec.script,
        &build_dir,
        false, // verbose
        &log_path,
        Some(target_arch),
    )?;

    // 6b. Relink (make relocatable)
    // Build-from-source binaries often have absolute paths to the Sysroot in their RPATH.
    // We patch these to be relative to the binary/dylib so the package stays portable.
    if cfg!(target_os = "macos") {
        crate::relinker::Relinker::relink_all(&build_dir)?;
    }

    // 7. Bundle Output (tar.zst)
    let bundle_path = tmp_dir.path().join("bundle.tar.zst");
    bundle_directory(&build_dir, &bundle_path)?;

    // 8. Compute Hash and Upload to Artifact Store (R2)
    let bundle_data = std::fs::read(&bundle_path)?;
    let mut hasher = sha2::Sha256::new();
    hasher.update(&bundle_data);
    let hash_hex = format!("{:x}", hasher.finalize());

    let mirror_url = store.upload_chunked(&hash_hex, &bundle_data).await?;
    tracing::debug!("      Uploaded to mirror (chunked): {mirror_url}");

    // 9. Return VersionInfo pointing to the hydrated binary
    Ok(VersionInfo {
        version: display_version.to_string(),
        binaries: vec![IndexBinary {
            arch: target_arch,
            url: mirror_url,
            hash: Sha256Hash::new(hash_hex),
            hash_type: HashType::Sha256,
        }],
        source: None, // Consumer only sees the binary
        deps: Vec::new(),
        build_deps: build_spec.dependencies.clone(),
        build_script: build_spec.script.clone(),
        bin: template
            .install
            .bin
            .clone()
            .unwrap_or_else(|| vec![template.package.name.to_string()]),
        hints: template.hints.post_install.clone(),
        app: template.install.app.clone(),
    })
}

/// Downloads an asset from a URL and uploads it to the artifact store using chunking.
async fn mirror_asset(client: &Client, url: &str, hash: &str, store: &ArtifactStore) -> Result<()> {
    // Skip if artifact already mirrored (optimization)
    if store.exists_manifest(hash).await {
        return Ok(());
    }

    let resp = client.get(url).send().await?.error_for_status()?;
    let data = resp.bytes().await?;

    store.upload_chunked(hash, &data).await?;
    Ok(())
}

/// Helper to bundle a directory into a .tar.zst archive for the artifact store.
///
/// Symlinks are preserved as symlinks in the archive rather than being
/// dereferenced. This avoids ENOENT errors from dangling absolute symlinks
/// (common after `make install` with `$PREFIX`) and ensures the archive is
/// portable.
fn bundle_directory(src_dir: &Path, dest_archive: &Path) -> Result<()> {
    use std::fs::File;
    use std::io::BufWriter;

    let file = File::create(dest_archive)?;
    let writer = BufWriter::new(file);
    let zstd_encoder = zstd::stream::Encoder::new(writer, 3)?;
    let mut tar_builder = tar::Builder::new(zstd_encoder);

    // Preserve symlinks instead of following them. Build outputs often contain
    // symlinks (e.g. bzegrep -> bzgrep) that should remain as links in the
    // archive. Following them would fail for broken absolute symlinks and
    // would duplicate file content for valid ones.
    tar_builder.follow_symlinks(false);

    tar_builder.append_dir_all(".", src_dir)?;
    tar_builder.finish()?;
    tar_builder.into_inner()?.finish()?;

    Ok(())
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
    use forges::traits::{AssetInfo, ReleaseInfo};
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
                homepage: String::new(),
                license: String::new(),
                tags: vec![],
            },
            discovery: DiscoveryConfig::GitHub {
                github: "owner/repo".to_string(),
                tag_pattern: "v{{version}}".to_string(),
                include_prereleases: false,
            },
            assets: AssetConfig {
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
            source: None,
            build: None,
            dependencies: crate::package::Dependencies::default(),
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

        let index = PackageIndex::new();
        // Attempt to hydrate v1.0.0
        let ctx = IndexingContext {
            client: &client,
            hash_cache,
            releases_map,
            index: &index,
        };
        // Attempt to hydrate v1.0.0
        let result = package_to_index_ver(
            ctx, &template, "v1.0.0", // full_tag for map lookup
            "1.0.0",  // url_version for templates
            "1.0.0",  // display_version
        )
        .await;

        // SHOULD NO LONGER BAIL. It should succeed but with empty binaries.
        // This allows indexing metadata-only versions (e.g. for dependency resolution).
        assert!(result.is_ok());
        let info = result.unwrap();
        assert_eq!(info.version, "1.0.0");
        assert!(info.binaries.is_empty());
        println!("Test success: Version indexed successfully with empty binaries (Metadata-only).");
    }

    #[tokio::test]
    async fn test_dependency_population() {
        let client = Client::new();
        let hash_cache = Arc::new(Mutex::new(HashCache::default()));

        let mut select = HashMap::new();
        select.insert(
            "universal-macos".to_string(),
            crate::package::AssetSelector::Suffix {
                suffix: "1.0.0".to_string(),
            },
        );

        let template = PackageTemplate {
            package: crate::package::PackageInfoTemplate {
                name: "test-pkg".into(),
                description: "test".to_string(),
                homepage: String::new(),
                license: String::new(),
                tags: vec![],
            },
            discovery: DiscoveryConfig::Manual {
                manual: vec!["1.0.0".to_string()],
            },
            assets: AssetConfig {
                select,
                skip_checksums: true,
                checksum_url: None,
            },
            source: None,
            build: None,
            dependencies: crate::package::Dependencies {
                runtime: vec!["runtime-dep".to_string()],
                build: vec!["build-dep".to_string()],
                optional: vec![],
            },
            install: InstallSpec::default(),
            hints: crate::package::Hints::default(),
        };

        let ctx = IndexingContext {
            client: &client,
            index: &PackageIndex::new(),
            hash_cache: hash_cache.clone(),
            releases_map: None,
        };

        // Pre-populate hash cache to avoid network call on empty URL (which causes panic)
        hash_cache
            .lock()
            .await
            .insert(String::new(), "dummy_hash".to_string(), HashType::Sha256);

        let ver_info = package_to_index_ver(ctx, &template, "v1.0.0", "1.0.0", "1.0.0")
            .await
            .unwrap();

        assert_eq!(ver_info.deps, vec!["runtime-dep"]);
        assert_eq!(ver_info.build_deps, vec!["build-dep"]);
    }
}
