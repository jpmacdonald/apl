//! `apl-builder` - The APL Build System.
//!
//! This binary discovers, builds, and indexes ports from various sources
//! (GitHub, `HashiCorp`, etc.) and uploads artifacts to the R2 store.

use anyhow::{Context, Result};
use apl_schema::Artifact;
use apl_schema::{PortConfig, PortManifest};
use clap::Parser;
use glob::glob;
use opendal::{Operator, services::S3};
use std::path::PathBuf;
use tokio::fs;

pub use apl_core::Strategy;
pub use apl_core::strategies::{
    AwsStrategy, BuildStrategy, GolangStrategy, HashiCorpStrategy, NodeStrategy,
};

use apl_core::indexer::hydrate_from_source;
use apl_core::package::{PackageInfoTemplate, PackageTemplate};
use apl_schema::Arch;

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
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
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

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
                println!("Rosetta 2 detected -- enabling x86_64 cross-builds.");
            } else {
                println!("Rosetta 2 not available -- building arm64 only.");
            }
        }
        archs
    };
    println!(
        "Target architectures: {}",
        target_archs
            .iter()
            .map(Arch::as_str)
            .collect::<Vec<_>>()
            .join(", ")
    );

    // Create output directory
    fs::create_dir_all(&args.output_dir).await?;

    // Initialize R2 operator. In dry-run mode the store is optional since
    // no uploads or remote reads are performed.
    let mut builder = S3::default();

    if !args.dry_run {
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
    }

    builder.region("auto");

    let op = Operator::new(builder)?.finish();

    // 1. Discover and load all port manifests
    let pattern = args.ports_dir.join("*").join("*.toml");
    let pattern_str = pattern.to_str().context("Invalid path pattern")?;
    let mut manifests = std::collections::HashMap::new();

    println!("Discovering ports in {}...", args.ports_dir.display());
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
            Err(e) => return Err(e).with_context(|| format!("Failed to parse {}", path.display())),
        };

        manifests.insert(
            apl_schema::types::PackageName::new(&manifest.package.name),
            manifest,
        );
    }

    println!("Found {} port manifests.", manifests.len());
    for (name, manifest) in &manifests {
        if let PortConfig::Build { spec, .. } = &manifest.package.config {
            println!("  Port: {}, Deps: {:?}", name, spec.dependencies);
        } else {
            println!("  Port: {name} (not a build strategy)");
        }
    }

    // 2. Build a temporary index for dependency resolution
    let mut index = apl_schema::PackageIndex::new();
    for manifest in manifests.values() {
        let mut entry = apl_schema::index::IndexEntry {
            name: manifest.package.name.clone(),
            ..Default::default()
        };

        // We only need the dependency info for resolution
        let mut release = apl_schema::index::VersionInfo {
            version: "0.0.0".to_string(), // Dummy version for resolution
            ..Default::default()
        };

        if let PortConfig::Build { spec, .. } = &manifest.package.config {
            release.deps.clone_from(&spec.dependencies);
            release.build_deps.clone_from(&spec.dependencies); // In ports, deps are usually build-time deps
        }

        entry.releases.push(release);
        index.upsert(entry);
    }

    // 3. Resolve build plan (topological layers)
    let layers = apl_core::resolver::resolve_build_plan(&index)
        .context("Failed to resolve topological build order")?;

    println!("Build Plan: {} layers", layers.len());
    for (i, layer) in layers.iter().enumerate() {
        let layer_names: Vec<String> = layer.iter().map(std::string::ToString::to_string).collect();
        println!("  Layer {}: {}", i, layer_names.join(", "));
    }

    let mut failed_ports = Vec::new();

    // 4. Execute build plan layer by layer
    for layer in layers {
        for port_name in layer {
            // Respect filter if provided
            if let Some(filter) = &args.filter {
                if port_name.as_str() != filter {
                    continue;
                }
            }

            let Some(manifest) = manifests.get(&port_name) else {
                continue;
            };

            println!("\n[Layer] Processing port: {port_name}");

            if let Err(e) = process_port(&args, &op, manifest, &index, &target_archs).await {
                eprintln!("  ERROR processing {port_name}: {e:#}");
                failed_ports.push(port_name.to_string());
            }
        }
    }

    Ok(())
}

/// Process a single port manifest end-to-end: fetch artifacts, validate,
/// merge with existing index, and upload. Extracted so that failures are
/// isolated per-port via the `?` operator.
async fn process_port(
    args: &Args,
    op: &Operator,
    manifest: &PortManifest,
    index: &apl_schema::PackageIndex,
    target_archs: &[Arch],
) -> Result<()> {
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

    // Fetch existing index for incremental updates
    let r2_path = format!("ports/{port_name}/index.json");
    let cache_path = format!("ports/{port_name}/cache.json");

    let existing_artifacts = if let Ok(arts) = fetch_existing_index(op, &r2_path).await {
        println!(
            "  [Incremental] Loaded {} existing artifacts from R2",
            arts.len()
        );
        arts
    } else {
        println!("  [Incremental] No existing index found, starting fresh.");
        Vec::new()
    };

    // Load strategy cache
    let mut cache: apl_core::strategies::StrategyCache =
        fetch_cache(op, &cache_path).await.unwrap_or_default();

    // Build known versions set
    let known_versions: std::collections::HashSet<String> = existing_artifacts
        .iter()
        .map(|a| a.version.clone())
        .collect();

    // Execute strategy with known versions and cache
    let raw_artifacts = strategy
        .fetch_artifacts(&known_versions, &mut cache)
        .await?;
    println!("  Found {} new artifact candidates.", raw_artifacts.len());

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

            // Set dependencies from the manifest
            if let PortConfig::Build { spec, .. } = &manifest.package.config {
                template.dependencies.build.clone_from(&spec.dependencies);
            }

            // Build for each target architecture independently.
            // Per-arch failures are logged but do not block other archs.
            for &arch in target_archs {
                println!(
                    "  [BFS] Building {} v{} ({})...",
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
                            "  WARN: {} v{} ({}) build failed: {e:#}",
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

    // Validate new artifacts
    let mut error_count = 0;
    for artifact in &artifacts {
        if let Err(e) = artifact.validate() {
            error_count += 1;
            if error_count <= 5 {
                eprintln!("  SKIP: {} v{} - {}", artifact.name, artifact.version, e);
            } else if error_count == 6 {
                eprintln!("  ... (suppressing further validation errors)");
            }
        }
    }

    let valid_new_artifacts: Vec<_> = artifacts
        .into_iter()
        .filter(|a| a.validate().is_ok())
        .collect();

    if error_count > 0 {
        println!(
            "  {} valid new, {} skipped (missing checksums)",
            valid_new_artifacts.len(),
            error_count
        );
    } else {
        println!(
            "  {} valid new artifacts ready for index.",
            valid_new_artifacts.len()
        );
    }

    // Merge and deduplicate
    // Key: (version, arch, sha256). New artifacts win on collision.
    let mut merged_map = std::collections::HashMap::new();

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

    println!(
        "  [Incremental] Final index contains {} artifacts.",
        final_artifacts.len()
    );

    // Serialize and upload
    let index_json = serde_json::to_vec_pretty(&final_artifacts)?;
    let cache_json = serde_json::to_vec_pretty(&cache)?;

    let local_path = args.output_dir.join(port_name).join("index.json");
    let local_cache_path = args.output_dir.join(port_name).join("cache.json");

    if let Some(parent) = local_path.parent() {
        fs::create_dir_all(parent).await?;
    }
    fs::write(&local_path, &index_json).await?;
    fs::write(&local_cache_path, &cache_json).await?;
    println!("  Written to local: {}", local_path.display());

    if args.dry_run {
        println!(
            "  [Dry Run] Would upload ports/{port_name}/index.json ({} bytes)",
            index_json.len()
        );
        return Ok(());
    }

    op.write(&r2_path, index_json).await?;
    op.write(&cache_path, cache_json).await?;
    println!("  Uploaded to {r2_path} and {cache_path}");

    Ok(())
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
