//! `apl-builder` - The APL Build System.
//!
//! This binary discovers, builds, and indexes ports from various sources
//! (GitHub, `HashiCorp`, etc.) and uploads artifacts to the R2 store.

use anyhow::{Context, Result};
use apl_schema::Artifact;
use apl_schema::{PortConfig, PortManifest};
use clap::Parser;
use futures::future::join_all;
use glob::glob;
use opendal::{Operator, services::S3};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::fs;

pub use apl_core::Strategy;
pub use apl_core::strategies::{
    AwsStrategy, BuildStrategy, GolangStrategy, HashiCorpStrategy, NodeStrategy,
};

use apl_core::indexer::hydrate_from_source;
use apl_core::package::{PackageInfoTemplate, PackageTemplate};
use apl_schema::Arch;

#[derive(Parser, Debug)]
#[command(author, version, about = "Algorithmic Harvester for APL Ports", long_about = None)]
struct Args {
    /// Path to the ports directory (defaults to "ports")
    #[arg(short, long, default_value = "ports")]
    ports_dir: PathBuf,

    /// Filter to run a specific port
    #[arg(short, long)]
    filter: Option<String>,

    /// Output directory for index artifacts (defaults to "output")
    #[arg(short, long, default_value = "output")]
    output_dir: PathBuf,

    /// Dry run mode (don't upload to R2)
    #[arg(long, default_value_t = false)]
    dry_run: bool,

    /// Restrict builds to a single architecture (arm64 or `x86_64`).
    /// By default, builds for all supported architectures on the host.
    #[arg(long)]
    arch: Option<Arch>,

    /// Maximum parallel builds per layer (default: 4)
    #[arg(long, default_value_t = 4)]
    parallel: usize,
}

/// Result of processing a single port
struct PortResult {
    name: String,
    status: PortStatus,
    duration: Duration,
}

enum PortStatus {
    Built { artifacts: usize },
    Skipped,
    Failed,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let start_time = Instant::now();

    // Determine which architectures to build for.
    let target_archs: Vec<Arch> = if let Some(arch) = args.arch {
        vec![arch]
    } else {
        let mut archs = vec![Arch::current()];
        // On Apple Silicon, probe Rosetta 2 for x86_64 cross-compilation.
        if Arch::current() == Arch::Arm64 {
            let rosetta_ok = std::process::Command::new("arch")
                .args(["-x86_64", "/usr/bin/true"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false);

            if rosetta_ok {
                archs.push(Arch::X86_64);
                println!("  rosetta detected, x86_64 cross-builds enabled");
            } else {
                println!("  rosetta unavailable, arm64 only");
            }
        }
        archs
    };
    println!(
        "  targets: {}",
        target_archs
            .iter()
            .map(Arch::as_str)
            .collect::<Vec<_>>()
            .join(", ")
    );

    // Create output directory
    fs::create_dir_all(&args.output_dir).await?;

    // Initialize R2 operator only when not in dry-run mode.
    let op: Option<Arc<Operator>> = if args.dry_run {
        None
    } else {
        let mut builder = S3::default();
        builder.bucket(
            &std::env::var("APL_ARTIFACT_STORE_BUCKET")
                .context("APL_ARTIFACT_STORE_BUCKET must be set")?,
        );
        builder.endpoint(
            &std::env::var("APL_ARTIFACT_STORE_ENDPOINT")
                .context("APL_ARTIFACT_STORE_ENDPOINT must be set")?,
        );
        builder.access_key_id(
            &std::env::var("APL_ARTIFACT_STORE_ACCESS_KEY")
                .context("APL_ARTIFACT_STORE_ACCESS_KEY must be set")?,
        );
        builder.secret_access_key(
            &std::env::var("APL_ARTIFACT_STORE_SECRET_KEY")
                .context("APL_ARTIFACT_STORE_SECRET_KEY must be set")?,
        );
        builder.region("auto");
        Some(Arc::new(Operator::new(builder)?.finish()))
    };

    // Discover and load all port manifests
    let pattern = args.ports_dir.join("*").join("*.toml");
    let pattern_str = pattern.to_str().context("Invalid path pattern")?;
    let mut manifests = HashMap::new();

    println!("  discovering ports in {}", args.ports_dir.display());
    for entry in glob(pattern_str)? {
        let path = entry?;
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if file_name == "Cargo.toml" || file_name.starts_with('.') {
            continue;
        }

        let content = fs::read_to_string(&path).await?;
        let manifest: PortManifest = match toml::from_str(&content) {
            Ok(m) => m,
            Err(_) if !content.contains("[package]") => continue,
            Err(e) => return Err(e).with_context(|| format!("failed to parse {}", path.display())),
        };

        manifests.insert(
            apl_schema::types::PackageName::new(&manifest.package.name),
            manifest,
        );
    }

    println!("  found {} ports", manifests.len());
    for (name, manifest) in &manifests {
        if let PortConfig::Build { spec, .. } = &manifest.package.config {
            println!("    {name} deps: {:?}", spec.dependencies);
        } else {
            println!("    {name} (prebuilt)");
        }
    }

    // Build a temporary index for dependency resolution
    let mut index = apl_schema::PackageIndex::new();
    for manifest in manifests.values() {
        let mut entry = apl_schema::index::IndexEntry {
            name: manifest.package.name.clone(),
            ..Default::default()
        };

        let mut release = apl_schema::index::VersionInfo {
            version: "0.0.0".to_string(),
            ..Default::default()
        };

        if let PortConfig::Build { spec, .. } = &manifest.package.config {
            release.deps.clone_from(&spec.dependencies);
            release.build_deps.clone_from(&spec.dependencies);
        }

        entry.releases.push(release);
        index.upsert(entry);
    }

    // Resolve build plan (topological layers)
    let layers = apl_core::resolver::resolve_build_plan(&index)
        .context("Failed to resolve topological build order")?;

    println!("  build plan: {} layers", layers.len());
    for (i, layer) in layers.iter().enumerate() {
        let layer_names: Vec<String> = layer.iter().map(std::string::ToString::to_string).collect();
        println!("    {i}: {}", layer_names.join(", "));
    }

    let mut all_results: Vec<PortResult> = Vec::new();
    let index = Arc::new(index);
    let manifests = Arc::new(manifests);

    // Execute build plan layer by layer, with parallelism within each layer
    for layer in layers {
        let ports_to_build: Vec<_> = layer
            .into_iter()
            .filter(|port_name| {
                if let Some(filter) = &args.filter {
                    port_name.as_str() == filter
                } else {
                    true
                }
            })
            .filter(|port_name| manifests.contains_key(port_name))
            .collect();

        if ports_to_build.is_empty() {
            continue;
        }

        // Process ports in parallel chunks
        for chunk in ports_to_build.chunks(args.parallel) {
            let futures: Vec<_> = chunk
                .iter()
                .map(|port_name| {
                    let port_name = port_name.clone();
                    let manifest = manifests.get(&port_name).unwrap().clone();
                    let op = op.clone();
                    let index = index.clone();
                    let target_archs = target_archs.clone();
                    let output_dir = args.output_dir.clone();
                    let dry_run = args.dry_run;

                    async move {
                        let port_start = Instant::now();
                        println!();
                        println!("  processing {port_name}");

                        let result = process_port(
                            &output_dir,
                            dry_run,
                            op.as_deref(),
                            &manifest,
                            &index,
                            &target_archs,
                        )
                        .await;

                        let duration = port_start.elapsed();

                        match result {
                            Ok(artifact_count) => PortResult {
                                name: port_name.to_string(),
                                status: if artifact_count > 0 {
                                    PortStatus::Built {
                                        artifacts: artifact_count,
                                    }
                                } else {
                                    PortStatus::Skipped
                                },
                                duration,
                            },
                            Err(e) => {
                                eprintln!("    error: {port_name}: {e:#}");
                                PortResult {
                                    name: port_name.to_string(),
                                    status: PortStatus::Failed,
                                    duration,
                                }
                            }
                        }
                    }
                })
                .collect();

            let results = join_all(futures).await;
            all_results.extend(results);
        }
    }

    // Print summary
    let total_duration = start_time.elapsed();
    let built: Vec<_> = all_results
        .iter()
        .filter(|r| matches!(r.status, PortStatus::Built { .. }))
        .collect();
    let skipped: Vec<_> = all_results
        .iter()
        .filter(|r| matches!(r.status, PortStatus::Skipped))
        .collect();
    let failed: Vec<_> = all_results
        .iter()
        .filter(|r| matches!(r.status, PortStatus::Failed))
        .collect();

    println!();
    println!("  summary");

    if !built.is_empty() {
        for r in &built {
            if let PortStatus::Built { artifacts } = r.status {
                println!(
                    "    built {} ({} artifacts, {:.1}s)",
                    r.name,
                    artifacts,
                    r.duration.as_secs_f64()
                );
            }
        }
    }

    if !failed.is_empty() {
        for r in &failed {
            println!("    failed {}", r.name);
        }
    }

    println!();
    println!(
        "  {} built, {} skipped, {} failed in {:.1}s",
        built.len(),
        skipped.len(),
        failed.len(),
        total_duration.as_secs_f64()
    );

    if !failed.is_empty() {
        std::process::exit(1);
    }

    Ok(())
}

/// Process a single port manifest end-to-end. Returns the number of new artifacts built.
async fn process_port(
    output_dir: &Path,
    dry_run: bool,
    op: Option<&Operator>,
    manifest: &PortManifest,
    index: &apl_schema::PackageIndex,
    target_archs: &[Arch],
) -> Result<usize> {
    let port_name = &manifest.package.name;

    // Instantiate the discovery strategy from the manifest config.
    let strategy: Box<dyn Strategy> = match &manifest.package.config {
        PortConfig::HashiCorp { product } => Box::new(HashiCorpStrategy::new(product.clone())),
        PortConfig::GitHub { .. } => {
            anyhow::bail!("GitHub port strategy is not implemented -- use a Build port instead");
        }
        PortConfig::Golang => Box::new(GolangStrategy),
        PortConfig::Node => Box::new(NodeStrategy),
        PortConfig::Aws => Box::new(AwsStrategy),
        PortConfig::Build { source_url, spec } => Box::new(BuildStrategy::new(
            port_name.to_string(),
            source_url.clone(),
            Some(spec.tag_pattern.clone()),
            spec.clone(),
        )),
        _ => {
            anyhow::bail!("strategy not implemented yet");
        }
    };

    // Fetch existing index for incremental updates (skip in dry-run mode)
    let r2_path = format!("ports/{port_name}/index.json");
    let cache_path = format!("ports/{port_name}/cache.json");

    let existing_artifacts = if let Some(operator) = op {
        if let Ok(arts) = fetch_existing_index(operator, &r2_path).await {
            println!("    loaded {} existing artifacts", arts.len());
            arts
        } else {
            println!("    no existing index, starting fresh");
            Vec::new()
        }
    } else {
        println!("    dry-run: skipping R2 fetch");
        Vec::new()
    };

    // Load strategy cache (skip in dry-run mode)
    let mut cache: apl_core::strategies::StrategyCache = if let Some(operator) = op {
        fetch_cache(operator, &cache_path).await.unwrap_or_default()
    } else {
        apl_core::strategies::StrategyCache::default()
    };

    // Build known versions set (for strategy filtering)
    let known_versions: HashSet<String> = existing_artifacts
        .iter()
        .map(|a| a.version.clone())
        .collect();

    // Build known (version, arch) set for per-arch watermarking
    let known_version_archs: HashSet<(String, String)> = existing_artifacts
        .iter()
        .map(|a| (a.version.clone(), a.arch.clone()))
        .collect();

    let raw_artifacts = strategy
        .fetch_artifacts(&known_versions, &mut cache)
        .await?;
    println!("    {} new candidates", raw_artifacts.len());

    let mut artifacts = Vec::new();
    let client = reqwest::Client::new();
    let store = apl_core::io::artifacts::get_artifact_store()
        .await
        .context("Failed to initialize artifact store")?;

    for art in raw_artifacts {
        if art.arch == "source" {
            let mut template = PackageTemplate {
                package: PackageInfoTemplate {
                    name: apl_core::types::PackageName::from(art.name.as_str()),
                    description: String::new(),
                    homepage: String::new(),
                    license: String::new(),
                    tags: Vec::new(),
                },
                discovery: apl_core::package::DiscoveryConfig::Manual {
                    manual: vec![art.version.clone()],
                },
                assets: apl_core::package::AssetConfig::default(),
                source: Some(apl_core::package::SourceTemplate {
                    url: art.url.clone(),
                    format: apl_schema::ArtifactFormat::TarGz,
                    sha256: None,
                }),
                build: match &manifest.package.config {
                    PortConfig::Build { spec, .. } => Some(spec.clone()),
                    _ => None,
                },
                dependencies: apl_core::package::Dependencies::default(),
                install: apl_core::package::InstallSpec::default(),
                hints: apl_core::package::Hints::default(),
            };

            if let PortConfig::Build { spec, .. } = &manifest.package.config {
                template.dependencies.build.clone_from(&spec.dependencies);
            }

            for &arch in target_archs {
                // Per-arch watermark: skip if (version, arch) already exists
                let key = (art.version.clone(), arch.to_string());
                if known_version_archs.contains(&key) {
                    println!(
                        "    skipping {} {} ({}) - already built",
                        art.name,
                        art.version,
                        arch.as_str()
                    );
                    continue;
                }

                let build_start = Instant::now();
                println!(
                    "    building {} {} ({})",
                    art.name,
                    art.version,
                    arch.as_str()
                );

                match hydrate_from_source(
                    &client,
                    &template,
                    &art.version,
                    &art.version,
                    template.build.as_ref().unwrap(),
                    &store,
                    index,
                    arch,
                )
                .await
                {
                    Ok(version_info) => {
                        let build_duration = build_start.elapsed();
                        println!(
                            "    built {} {} ({}) in {:.1}s",
                            art.name,
                            art.version,
                            arch.as_str(),
                            build_duration.as_secs_f64()
                        );
                        for bin in version_info.binaries {
                            artifacts.push(Artifact {
                                name: art.name.clone(),
                                version: art.version.clone(),
                                arch: bin.arch.to_string(),
                                url: bin.url,
                                sha256: bin.hash.to_string(),
                            });
                        }
                    }
                    Err(e) => {
                        eprintln!(
                            "    warn: {} {} ({}) failed: {e:#}",
                            art.name,
                            art.version,
                            arch.as_str()
                        );
                    }
                }
            }
        } else {
            artifacts.push(art);
        }
    }

    let mut error_count = 0;
    for artifact in &artifacts {
        if let Err(e) = artifact.validate() {
            error_count += 1;
            if error_count <= 5 {
                eprintln!("    skip: {} {} - {}", artifact.name, artifact.version, e);
            } else if error_count == 6 {
                eprintln!("    ... (suppressing further errors)");
            }
        }
    }

    let valid_new_artifacts: Vec<_> = artifacts
        .into_iter()
        .filter(|a| a.validate().is_ok())
        .collect();

    let new_artifact_count = valid_new_artifacts.len();

    if error_count > 0 {
        println!("    {new_artifact_count} valid, {error_count} skipped");
    } else if new_artifact_count > 0 {
        println!("    {new_artifact_count} valid artifacts");
    }

    // Merge and deduplicate
    let mut merged_map = HashMap::new();

    for art in existing_artifacts {
        let key = (art.version.clone(), art.arch.clone(), art.sha256.clone());
        merged_map.insert(key, art);
    }
    for art in valid_new_artifacts {
        let key = (art.version.clone(), art.arch.clone(), art.sha256.clone());
        merged_map.insert(key, art);
    }

    let mut final_artifacts: Vec<_> = merged_map.into_values().collect();
    final_artifacts.sort_by(|a, b| b.version.cmp(&a.version));

    println!("    final index: {} artifacts", final_artifacts.len());

    // Serialize and upload
    let index_json = serde_json::to_vec_pretty(&final_artifacts)?;
    let cache_json = serde_json::to_vec_pretty(&cache)?;

    let local_path = output_dir.join(port_name).join("index.json");
    let local_cache_path = output_dir.join(port_name).join("cache.json");

    if let Some(parent) = local_path.parent() {
        fs::create_dir_all(parent).await?;
    }
    fs::write(&local_path, &index_json).await?;
    fs::write(&local_cache_path, &cache_json).await?;
    println!("    wrote {}", local_path.display());

    if dry_run || op.is_none() {
        println!(
            "    dry run: would upload {} ({} bytes)",
            r2_path,
            index_json.len()
        );
        return Ok(new_artifact_count);
    }

    let operator = op.expect("operator should be Some when not in dry-run");
    operator.write(&r2_path, index_json).await?;
    operator.write(&cache_path, cache_json).await?;
    println!("    uploaded {r2_path}");

    Ok(new_artifact_count)
}

async fn fetch_existing_index(op: &Operator, path: &str) -> Result<Vec<Artifact>> {
    let data = op.read(path).await?;
    let artifacts: Vec<Artifact> = serde_json::from_slice(&data)?;
    Ok(artifacts)
}

async fn fetch_cache(op: &Operator, path: &str) -> Result<apl_core::strategies::StrategyCache> {
    let data = op.read(path).await?;
    let cache: apl_core::strategies::StrategyCache = serde_json::from_slice(&data)?;
    Ok(cache)
}
