use anyhow::{Context, Result};
use apl_types::{PortManifest, PortConfig};
use clap::Parser;
use glob::glob;
use opendal::{Operator, services::S3};
use std::path::PathBuf;
use tokio::fs;

// Define strategies module (will be implemented next)
mod strategies;
use strategies::{HashiCorpStrategy, GitHubStrategy, GolangStrategy, NodeStrategy, AwsStrategy, PythonStrategy, RubyStrategy};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to the ports directory (defaults to "ports")
    #[arg(short, long, default_value = "ports")]
    ports_dir: PathBuf,

    /// Filter to run a specific port
    #[arg(short, long)]
    filter: Option<String>,
    
    /// Dry run mode (don't upload to R2)
    #[arg(long, default_value_t = false)]
    dry_run: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    
    // Initialize R2 Operator
    let builder = S3::default()
        .bucket("apl-store") // Env var overrides: APL_ARTIFACT_STORE_BUCKET
        .endpoint("https://b32f5efef56e1b61c8ef5a2c77f07fbb.r2.cloudflarestorage.com")
        .region("auto");
    
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
        let strategy: Box<dyn apl_ports::Strategy> = match &manifest.package.config {
            PortConfig::HashiCorp { product } => {
                Box::new(HashiCorpStrategy::new(product.clone()))
            },
            PortConfig::GitHub { owner, repo } => {
                Box::new(GitHubStrategy::new(owner.clone(), repo.clone()))
            },
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
        
        // Validation with nice error reporting
        for (i, artifact) in artifacts.iter().enumerate() {
            if let Err(_e) = artifact.validate() {
               eprintln!("  [ERROR] Artifact {}/{} ({}) is invalid: {}", 
                   i+1, artifacts.len(), artifact.version, _e);
               // We could panic/exit here, or filter them out.
               // For strictness, let's filter but warn loudly? 
               // Or fail the whole port? User said "catch things... GUARANTEE validity".
               // So if one is invalid, the index is corrupt?
               // Let's filter invalid ones for resilience but warn.
            }
        }
        
        let valid_artifacts: Vec<_> = artifacts.into_iter()
            .filter(|a| {
                if a.validate().is_err() {
                    // Already logged above
                    false
                } else {
                    true
                }
            })
            .collect();
            
        println!("  {} valid artifacts ready for index.", valid_artifacts.len());


        if args.dry_run {
            println!("  [Dry Run] Would upload ports/{port_name}/index.json");
            continue;
        }

        // Upload to R2
        let index_json = serde_json::to_vec_pretty(&valid_artifacts)?;
        let r2_path = format!("ports/{port_name}/index.json");
        op.write(&r2_path, index_json).await?;
        println!("  Uploaded to {r2_path}");
    }

    Ok(())
}
