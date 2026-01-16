use anyhow::{Context, Result};
use apl_schema::Artifact;
use apl_schema::{PortConfig, PortManifest};
use clap::Parser;
use glob::glob;
use opendal::{Operator, services::S3};
use std::path::PathBuf;
use tokio::fs;

// Use library crate
use apl_core::Strategy;
use apl_core::strategies::{
    AwsStrategy, GitHubStrategy, GolangStrategy, HashiCorpStrategy, NodeStrategy, PythonStrategy,
    RubyStrategy,
};

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

    // Find all port.toml files
    let pattern = args.ports_dir.join("**").join("port.toml");
    let pattern_str = pattern.to_str().context("Invalid path pattern")?;

    for entry in glob(pattern_str)? {
        let path = entry?;
        let parent_dir = path.parent().unwrap();
        let port_name = parent_dir.file_name().unwrap().to_str().unwrap();

        if let Some(filter) = &args.filter {
            if port_name != filter {
                continue;
            }
        }

        println!("Processing port: {port_name}");

        let content = fs::read_to_string(&path).await?;
        let manifest: PortManifest = toml::from_str(&content)
            .with_context(|| format!("Failed to parse {}", path.display()))?;

        // Instantiate Strategy
        let strategy: Box<dyn Strategy> = match &manifest.package.config {
            PortConfig::HashiCorp { product } => Box::new(HashiCorpStrategy::new(product.clone())),
            PortConfig::GitHub { owner, repo } => {
                Box::new(GitHubStrategy::new(owner.clone(), repo.clone()))
            }
            PortConfig::Golang => Box::new(GolangStrategy),
            PortConfig::Node => Box::new(NodeStrategy),
            PortConfig::Aws => Box::new(AwsStrategy),
            PortConfig::Python => Box::new(PythonStrategy),
            PortConfig::Ruby => Box::new(RubyStrategy),
            _ => {
                eprintln!("Strategy not implemented yet for {port_name}");
                continue;
            }
        };

        // Execute
        let artifacts = strategy.fetch_artifacts().await?;
        println!("  Found {} artifacts. Validating...", artifacts.len());

        // Validation with nice error reporting (limit noise)
        let mut error_count = 0;
        for (i, artifact) in artifacts.iter().enumerate() {
            if let Err(e) = artifact.validate() {
                error_count += 1;
                if error_count <= 5 {
                    eprintln!("  SKIP: {} v{} - {}", artifact.name, artifact.version, e);
                } else if error_count == 6 {
                    eprintln!("  ... (suppressing further validation errors)");
                }
            }
        }

        let valid_artifacts: Vec<_> = artifacts
            .into_iter()
            .filter(|a| a.validate().is_ok())
            .collect();

        if error_count > 0 {
            println!(
                "  {} valid, {} skipped (missing checksums)",
                valid_artifacts.len(),
                error_count
            );
        } else {
            println!(
                "  {} valid artifacts ready for index.",
                valid_artifacts.len()
            );
        }

        // Incremental Update Logic
        let r2_path = format!("ports/{port_name}/index.json");

        // 1. Fetch existing index
        let existing_artifacts = match fetch_existing_index(&op, &r2_path).await {
            Ok(arts) => {
                println!(
                    "  [Incremental] Loaded {} existing artifacts from R2",
                    arts.len()
                );
                arts
            }
            Err(_) => {
                println!("  [Incremental] No existing index found (or error), starting fresh.");
                Vec::new()
            }
        };

        // 2. Merge & Deduplicate
        // Key: (Version, Arch, SHA256) -> Artifact
        // We prefer the NEW artifact if there's a collision, as it was just validated.
        let mut merged_map = std::collections::HashMap::new();

        // Load old first
        for art in existing_artifacts {
            let key = (art.version.clone(), art.arch.clone(), art.sha256.clone());
            merged_map.insert(key, art);
        }

        // Overlay new
        for art in valid_artifacts {
            let key = (art.version.clone(), art.arch.clone(), art.sha256.clone());
            merged_map.insert(key, art);
        }

        let mut final_artifacts: Vec<_> = merged_map.into_values().collect();

        // Sort for deterministic output (Semantic versioning sort would be best, but simple string sort is okay for raw index)
        final_artifacts.sort_by(|a, b| b.version.cmp(&a.version));

        println!(
            "  [Incremental] Final index contains {} artifacts.",
            final_artifacts.len()
        );

        // 3. Upload
        let index_json = serde_json::to_vec_pretty(&final_artifacts)?;

        // Write to local output dir
        let local_path = args.output_dir.join(port_name).join("index.json");
        if let Some(parent) = local_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        fs::write(&local_path, &index_json).await?;
        println!("  Written to local: {}", local_path.display());

        if args.dry_run {
            println!(
                "  [Dry Run] Would upload ports/{port_name}/index.json (Size: {} bytes)",
                index_json.len()
            );
            continue;
        }

        op.write(&r2_path, index_json).await?;
        println!("  Uploaded to {r2_path}");
    }

    Ok(())
}

async fn fetch_existing_index(op: &Operator, path: &str) -> Result<Vec<Artifact>> {
    let data = op.read(path).await?;
    let artifacts: Vec<Artifact> = serde_json::from_slice(&data)?;
    Ok(artifacts)
}
