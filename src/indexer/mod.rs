pub mod discovery;
pub mod hashing;
pub mod sources;
pub mod walk;

pub use discovery::*;
pub use hashing::HashCache;
pub use walk::{registry_path, walk_registry_toml_files};

use crate::core::index::{HashType, IndexBinary, PackageIndex, VersionInfo};
use crate::package::{DiscoveryConfig, PackageTemplate};
use crate::types::PackageName;
use anyhow::Result;
use reqwest::Client;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

use sources::traits::{ListingSource, ReleaseInfo};

use crate::io::artifacts::{ArtifactStore, get_artifact_store};

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

    // Initialize artifact store (optional, only if configured)
    let artifact_store: Option<Arc<ArtifactStore>> = get_artifact_store().await;
    if artifact_store.is_some() {
        println!("   ☁️  Artifact Store enabled (will mirror to R2)");
    }

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

    // Set mirror_base_url if artifact store is configured
    if let Some(ref store) = artifact_store {
        // Get public base URL from config (strip /cas/ suffix if present)
        let base = store.public_url("").trim_end_matches("/cas/").to_string();
        index.mirror_base_url = Some(base);
    }

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

        if let DiscoveryConfig::GitHub {
            github,
            tag_pattern,
            ..
        } = &template.discovery
        {
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
                pkg_tag_pattern_map.insert(template.package.name.to_string(), tag_pattern.clone());
                pkg_source_map.insert(template.package.name.to_string(), source_key);
            }
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

    // Pass 3.5: Build Build Graph to determine order
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
                deps: vec![],
                build_deps: template
                    .build
                    .as_ref()
                    .map(|b| b.dependencies.clone())
                    .unwrap_or_default(),
                bin: vec![],
                hints: "".into(),
                app: None,
                source: None,
                build_script: "".into(),
            },
        );
    }
    let layers = crate::core::resolver::resolve_build_plan(&stub_index)?;

    // Map templates by name for lookup
    let template_map: HashMap<PackageName, (std::path::PathBuf, PackageTemplate)> = templates
        .iter()
        .map(|(p, t)| (t.package.name.clone(), (p.clone(), t.clone())))
        .collect();

    // Pass 4: Process each dirty package using cached metadata (Layered Hydration)
    println!("   Processing Packages...");
    use futures::stream::{self, StreamExt};
    let pkg_source_map = Arc::new(pkg_source_map);
    let master_release_cache = Arc::new(master_release_cache);

    let mut total_releases = 0;
    let mut fully_indexed = 0;
    let mut partial = 0;
    let mut failed = 0;

    for (layer_idx, layer) in layers.iter().enumerate() {
        let mut layer_templates = Vec::new();
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

        if layers.len() > 1 {
            println!(
                "   --- Layer {}/{} ({} packages) ---",
                layer_idx + 1,
                layers.len(),
                layer_templates.len()
            );
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

                                let extracted = discovery::extract_version_from_tag(
                                    &release.tag_name,
                                    tag_pattern,
                                );

                                if let Some(normalized) = discovery::auto_parse_version(&extracted)
                                {
                                    versions.push((
                                        release.tag_name.clone(),
                                        extracted,
                                        normalized,
                                    ));
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
                            let local_index_ref = index_ref.clone();
                            async move {
                                let ctx = IndexingContext {
                                    client: &client,
                                    hash_cache: hash_cache_clone,
                                    releases_map: releases_map_clone,
                                    index: &local_index_ref,
                                };
                                let res = package_to_index_ver(
                                    ctx,
                                    &template_clone,
                                    &tag_for_lookup,
                                    &extracted,
                                    &display_ver,
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

        let layer_results: Vec<_> = results_stream.collect().await;

        for (template, v_infos, errors) in layer_results {
            let pkg_name = template.package.name.to_string();
            let mut v_infos = v_infos;
            let success_count = v_infos.len();
            let total_versions = v_infos.len() + errors.len();
            total_releases += success_count;

            // Generate binary deltas if artifact store is enabled
            if let Err(e) =
                process_deltas(client, &pkg_name, &mut v_infos, artifact_store.as_deref()).await
            {
                tracing::warn!("   ⚠ Delta generation failed for {pkg_name}: {e}");
            }

            for ver_info in v_infos {
                // Infer type: App if .app is present, else Cli
                let kind = if ver_info.app.is_some() { "app" } else { "cli" };

                index.upsert_release(
                    &pkg_name,
                    &template.package.description,
                    kind,
                    template.package.tags.clone(),
                    ver_info,
                );
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

pub struct IndexingContext<'a> {
    pub client: &'a Client,
    pub hash_cache: Arc<Mutex<HashCache>>,
    pub releases_map: Option<Arc<HashMap<String, ReleaseInfo>>>,
    pub index: &'a PackageIndex,
}

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
        )
        .await
        {
            Ok(info) => return Ok(info),
            Err(e) => {
                anyhow::bail!("Build-from-Source failed for {display_version}: {e}");
            }
        }
    }

    let release_info = ctx
        .releases_map
        .as_ref()
        .and_then(|map| map.get(full_tag))
        .ok_or_else(|| anyhow::anyhow!("Release {full_tag} not found in map"))?;

    let mut binaries = Vec::new();

    // Strategy 1: Explicit selectors for each arch
    for (arch_name, selector) in &template.assets.select {
        if let Some(asset) = discovery::find_asset_by_selector(&release_info.assets, selector) {
            // Resolve hash
            let hash_res = resolve_hash(
                ctx.client,
                template,
                &asset.download_url,
                full_tag,
                ctx.hash_cache.clone(),
                ctx.releases_map.clone(),
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
                patches: vec![],
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
                    ctx.client,
                    template,
                    &asset.download_url,
                    full_tag,
                    ctx.hash_cache.clone(),
                    ctx.releases_map.clone(),
                )
                .await?;

                binaries.push(IndexBinary {
                    arch: crate::types::Arch::Universal,
                    url: asset.download_url.clone(),
                    hash: crate::types::Sha256Hash::new(hash),
                    hash_type: HashType::Sha256,
                    patches: vec![],
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

/// Generate binary deltas between adjacent versions of a package.
///
/// For each architecture, identifies the previous version and creates a zstd-dictionary patch.
async fn process_deltas(
    client: &Client,
    pkg_name: &str,
    v_infos: &mut [VersionInfo],
    store: Option<&ArtifactStore>,
) -> Result<()> {
    let store = match store {
        Some(s) => s,
        None => return Ok(()),
    };

    // Sort versions descending (newest first)
    v_infos.sort_by(|a, b| {
        match (
            semver::Version::parse(&a.version),
            semver::Version::parse(&b.version),
        ) {
            (Ok(va), Ok(vb)) => vb.cmp(&va),
            _ => b.version.cmp(&a.version),
        }
    });

    // We only generate deltas for the top few versions to keep index size sane.
    // Transitioning from V_n -> V_{n+1}.
    for i in 0..v_infos.len().min(5).saturating_sub(1) {
        let (new_ver_slice, old_ver_slice) = v_infos.split_at_mut(i + 1);
        let new_ver = &mut new_ver_slice[i];
        let old_ver = &old_ver_slice[0];

        for new_bin in &mut new_ver.binaries {
            // Find same architecture in old version
            if let Some(old_bin) = old_ver.binaries.iter().find(|b| b.arch == new_bin.arch) {
                // Skip if same hash (no change) or patch already exists
                if old_bin.hash == new_bin.hash
                    || new_bin.patches.iter().any(|p| p.from_hash == old_bin.hash)
                {
                    continue;
                }

                tracing::info!(
                    "   ⚡ Generating delta for {} {} -> {} ({})",
                    pkg_name,
                    old_ver.version,
                    new_ver.version,
                    new_bin.arch
                );

                // 1. Get old binary data
                let old_data = match store.get(old_bin.hash.as_ref()).await {
                    Ok(d) => d,
                    Err(_) => match client.get(&old_bin.url).send().await {
                        Ok(resp) => resp.bytes().await?.to_vec(),
                        Err(_) => continue,
                    },
                };

                // 2. Get new binary data
                let new_data = match store.get(new_bin.hash.as_ref()).await {
                    Ok(d) => d,
                    Err(_) => match client.get(&new_bin.url).send().await {
                        Ok(resp) => resp.bytes().await?.to_vec(),
                        Err(_) => continue,
                    },
                };

                // 3. Generate delta
                let patch = match crate::io::delta::generate_delta(&old_data, &new_data, 3) {
                    Ok(p) => p,
                    Err(e) => {
                        tracing::warn!("      Failed to generate delta: {e}");
                        continue;
                    }
                };

                // Only keep patch if it saves significant space (>20% or >100KB)
                if patch.len() >= new_data.len()
                    || (patch.len() as f64 / new_data.len() as f64) > 0.8
                {
                    tracing::debug!("      Patch not small enough, skipping");
                    continue;
                }

                // 4. Upload delta to R2
                use sha2::{Digest, Sha256};
                let mut hasher = Sha256::new();
                hasher.update(&patch);
                let patch_hash_hex = format!("{:x}", hasher.finalize());
                let patch_hash = crate::types::Sha256Hash::new(patch_hash_hex);

                let patch_key = format!("deltas/{}_{}.zst", old_bin.hash, new_bin.hash);

                let body = aws_sdk_s3::primitives::ByteStream::from(patch.clone());
                if let Err(e) = store
                    .upload_stream(&patch_key, body, Some(patch.len() as i64))
                    .await
                {
                    tracing::warn!("      Failed to upload delta: {e}");
                    continue;
                }

                // 5. Add to patches list
                new_bin.patches.push(crate::core::index::PatchInfo {
                    from_hash: old_bin.hash.clone(),
                    patch_hash,
                    patch_size: patch.len() as u64,
                });

                tracing::info!(
                    "      ✓ Delta created: {} KB (compression: {:.1}%)",
                    patch.len() / 1024,
                    (1.0 - (patch.len() as f64 / new_data.len() as f64)) * 100.0
                );
            }
        }
    }

    Ok(())
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

async fn hydrate_from_source(
    client: &Client,
    template: &PackageTemplate,
    full_tag: &str,
    display_version: &str,
    build_spec: &crate::core::package::BuildSpec,
    store: &ArtifactStore,
    index: &PackageIndex,
) -> Result<VersionInfo> {
    use crate::core::builder::Builder;
    use crate::core::sysroot::Sysroot;
    use sha2::Digest;

    // 1. Resolve Source URL
    let source_url = match &template.source {
        Some(s) => s
            .url
            .replace("{{tag}}", full_tag)
            .replace("{{version}}", display_version),
        None => {
            // Heuristic for GitHub
            if let DiscoveryConfig::GitHub { github, .. } = &template.discovery {
                format!("https://github.com/{github}/archive/refs/tags/{full_tag}.tar.gz")
            } else {
                anyhow::bail!("No source URL provided for build-from-source template");
            }
        }
    };

    println!("      🔨 Hydrating from source: {source_url}");

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
    crate::io::extract::extract_auto(&source_archive, &extract_dir)?;
    crate::io::extract::strip_components(&extract_dir)?;

    // 5. Resolve and download dependencies
    let mut build_deps = Vec::new();
    let mut dep_tmps = Vec::new();

    for dep_name in &build_spec.dependencies {
        // Find dependency in the index
        if let Some(entry) = index.find(dep_name) {
            if let Some(latest) = entry.latest() {
                // Find binary for current architecture
                #[cfg(target_arch = "aarch64")]
                let my_arch = crate::types::Arch::Arm64;
                #[cfg(target_arch = "x86_64")]
                let my_arch = crate::types::Arch::X86_64;
                #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
                let my_arch = crate::types::Arch::Universal;

                if let Some(bin) = latest
                    .binaries
                    .iter()
                    .find(|b| b.arch == my_arch || b.arch == crate::types::Arch::Universal)
                {
                    println!(
                        "      📦 Satisfying build dep: {} ({})",
                        dep_name, latest.version
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
                    crate::io::extract::extract_auto(&dep_archive, &dep_extract_dir)?;

                    build_deps.push((dep_name.clone(), dep_extract_dir));
                    dep_tmps.push(dep_tmp);
                }
            }
        }
    }

    // 6. Build in Sysroot
    let sysroot = Sysroot::new()?;
    let builder = Builder::new(&sysroot);
    let log_path = crate::build_log_path(template.package.name.as_ref(), display_version);

    builder.build(
        &extract_dir,
        &build_deps,
        &build_spec.script,
        &build_dir,
        false, // verbose
        &log_path,
    )?;

    // 7. Bundle Output (tar.zst)
    let bundle_path = tmp_dir.path().join("bundle.tar.zst");
    bundle_directory(&build_dir, &bundle_path)?;

    // 8. Compute Hash and Upload to Artifact Store (R2)
    let bundle_data = std::fs::read(&bundle_path)?;
    let mut hasher = sha2::Sha256::new();
    hasher.update(&bundle_data);
    let hash_hex = format!("{:x}", hasher.finalize());

    let mirror_url = store.upload(&hash_hex, bundle_data).await?;
    println!("      ☁️  Uploaded to mirror: {mirror_url}");

    // 9. Determine Arch
    #[cfg(target_arch = "aarch64")]
    let arch = crate::types::Arch::Arm64;
    #[cfg(target_arch = "x86_64")]
    let arch = crate::types::Arch::X86_64;
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    let arch = crate::types::Arch::Universal;

    // 10. Return VersionInfo pointing to our hydrated binary
    Ok(VersionInfo {
        version: display_version.to_string(),
        binaries: vec![IndexBinary {
            arch,
            url: mirror_url,
            hash: crate::types::Sha256Hash::new(hash_hex),
            hash_type: HashType::Sha256,
            patches: vec![],
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

/// Helper to bundle a directory into a .tar.zst archive for the artifact store.
fn bundle_directory(src_dir: &Path, dest_archive: &Path) -> Result<()> {
    use std::fs::File;
    use std::io::BufWriter;

    let file = File::create(dest_archive)?;
    let writer = BufWriter::new(file);
    let zstd_encoder = zstd::stream::Encoder::new(writer, 3)?;
    let mut tar_builder = tar::Builder::new(zstd_encoder);

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
                tags: vec![], // Added this line based on the instruction
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
            source: None,
            build: None,
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

        // SHOULD BAIL with "No supported binaries found" because the arm64 asset was skipped locally
        assert!(result.is_err());
        let err = result.err().unwrap().to_string();
        assert!(err.contains("No supported binaries found for version 1.0.0"));
        println!(
            "Test success: Version skipped gracefully because asset was missing from local map."
        );
    }
}
