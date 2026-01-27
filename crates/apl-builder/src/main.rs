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
    AwsStrategy, BuildStrategy, GitHubStrategy, GolangStrategy, HashiCorpStrategy, NodeStrategy,
};

use apl_core::indexer::hydrate_from_source;
use apl_core::package::{PackageInfoTemplate, PackageTemplate};
use apl_schema::index::VersionInfo;

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
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // Create output directory
    fs::create_dir_all(&args.output_dir).await?;

    // Initialize R2 Operator
    let mut builder = S3::default();

    // Use env vars if present, otherwise fall back to internal defaults (or empty)
    if let Ok(bucket) = std::env::var("APL_ARTIFACT_STORE_BUCKET") {
        builder.bucket(&bucket);
    } else {
        builder.bucket("apl-store");
    }

    if let Ok(endpoint) = std::env::var("APL_ARTIFACT_STORE_ENDPOINT") {
        builder.endpoint(&endpoint);
    } else {
        builder.endpoint("https://b32f5efef56e1b61c8ef5a2c77f07fbb.r2.cloudflarestorage.com");
    }

    if let Ok(access_key) = std::env::var("APL_ARTIFACT_STORE_ACCESS_KEY") {
        builder.access_key_id(&access_key);
    }

    if let Ok(secret_key) = std::env::var("APL_ARTIFACT_STORE_SECRET_KEY") {
        builder.secret_access_key(&secret_key);
    }

    builder.region("auto");

    let op = Operator::new(builder)?.finish();

    // Find port manifests (one level deep: <port-name>/<port-name>.toml)
    let pattern = args.ports_dir.join("*").join("*.toml");
    let pattern_str = pattern.to_str().context("Invalid path pattern")?;

    let mut failed_ports = Vec::new();

    for entry in glob(pattern_str)? {
        let path = entry?;
        let content = fs::read_to_string(&path).await?;
        let manifest: PortManifest = toml::from_str(&content)
            .with_context(|| format!("Failed to parse {}", path.display()))?;

        let port_name = &manifest.package.name;

        if let Some(filter) = &args.filter {
            if port_name != filter {
                continue;
            }
        }

        println!("Processing port: {port_name}");

        // Per-port error boundary: one broken port must not abort the entire run.
        if let Err(e) = process_port(&args, &op, &manifest).await {
            eprintln!("  ERROR processing {port_name}: {e:#}");
            failed_ports.push(port_name.clone());
        }
    }

    if !failed_ports.is_empty() {
        eprintln!(
            "\n{} port(s) failed: {}",
            failed_ports.len(),
            failed_ports.join(", ")
        );
        std::process::exit(1);
    }

    Ok(())
}

/// Process a single port manifest end-to-end: fetch artifacts, validate,
/// merge with existing index, and upload. Extracted so that failures are
/// isolated per-port via the `?` operator.
async fn process_port(args: &Args, op: &Operator, manifest: &PortManifest) -> Result<()> {
    let port_name = &manifest.package.name;

    // Instantiate the discovery strategy from the manifest config.
    let strategy: Box<dyn Strategy> = match &manifest.package.config {
        PortConfig::HashiCorp { product } => Box::new(HashiCorpStrategy::new(product.clone())),
        PortConfig::GitHub { owner, repo } => {
            Box::new(GitHubStrategy::new(owner.clone(), repo.clone()))
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

    // -- 1. Fetch existing index for incremental updates ---------------
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

    // -- 1b. Load cache ------------------------------------------------
    let mut cache: apl_core::strategies::StrategyCache =
        fetch_cache(op, &cache_path).await.unwrap_or_default();

    // -- 2. Build known versions set -----------------------------------
    let known_versions: std::collections::HashSet<String> = existing_artifacts
        .iter()
        .map(|a| a.version.clone())
        .collect();

    // -- 3. Execute strategy with known versions and cache -------------
    let raw_artifacts = strategy
        .fetch_artifacts(&known_versions, &mut cache)
        .await?;
    println!("  Found {} new artifact candidates.", raw_artifacts.len());

    let mut artifacts = Vec::new();
    let client = reqwest::Client::new();
    let store = apl_core::io::artifacts::get_artifact_store()
        .await
        .context("Failed to initialize artifact store")?;
    let dummy_index = apl_schema::index::PackageIndex::new();

    for art in raw_artifacts {
        if art.arch == "source" {
            println!(
                "  [BFS] Building {} v{} from source...",
                art.name, art.version
            );

            let template = PackageTemplate {
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

            let version_info: VersionInfo = hydrate_from_source(
                &client,
                &template,
                &art.version,
                &art.version,
                template.build.as_ref().unwrap(),
                &store,
                &dummy_index,
            )
            .await?;

            for bin in version_info.binaries {
                artifacts.push(Artifact {
                    name: art.name.clone(),
                    version: art.version.clone(),
                    arch: bin.arch.to_string(),
                    url: bin.url,
                    sha256: bin.hash.to_string(),
                });
            }
        } else {
            artifacts.push(art);
        }
    }

    // -- 4. Validate new artifacts -------------------------------------
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

    // -- 5. Merge & deduplicate ----------------------------------------
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

    // -- 6. Serialize and upload ---------------------------------------
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
