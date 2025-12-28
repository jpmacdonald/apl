//! dl - Distill Package Manager CLI

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

mod cmd;
mod core;

#[derive(Parser)]
#[command(name = "dl")]
#[command(author, version, about = "dl - A modern package manager for macOS")]
pub struct Cli {
    /// Show what would happen without making changes
    #[arg(long, global = true)]
    dry_run: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Install a package
    Install {
        /// Package name(s), optionally with version: pkg or pkg@1.0.0
        #[arg(required = true)]
        packages: Vec<String>,
        /// Only install packages pinned in dl.lock
        #[arg(long)]
        locked: bool,
    },
    /// Remove a package
    Remove {
        /// Package name(s)
        #[arg(required = true)]
        packages: Vec<String>,
    },
    /// List installed packages
    List,
    /// Show package info
    Info {
        /// Package name
        package: String,
    },
    /// Compute BLAKE3 hash of a file (for formula authoring)
    Hash {
        /// Files to hash
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    /// Generate or update dl.lock from installed packages
    Lock,
    /// Search available packages
    Search {
        /// Search query
        query: String,
    },
    /// Generate index.bin from formulas directory
    #[command(name = "generate-index")]
    GenerateIndex {
        /// Directory containing formula files
        #[arg(default_value = "formulas")]
        formulas_dir: PathBuf,
        /// Output file
        #[arg(default_value = "index.bin")]
        output: PathBuf,
    },
    /// Remove orphaned CAS blobs and temp files
    Clean,
    /// Update package index from CDN
    Update {
        /// CDN URL for index
        #[arg(long, env = "DL_INDEX_URL", default_value = "https://raw.githubusercontent.com/jpmacdonald/distill/main/index.bin")]
        url: String,
    },
    /// Upgrade installed packages to latest versions
    Upgrade {
        /// Specific packages to upgrade (or all if empty)
        packages: Vec<String>,
    },
    /// Formula management commands
    Formula {
        #[command(subcommand)]
        command: FormulaCommands,
    },
    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        shell: clap_complete::Shell,
    },
    /// Update dl itself to the latest version
    SelfUpdate,
    /// Run a package without installing it globally
    Run {
        /// Package name
        package: String,
        /// Arguments for the package
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

#[derive(Subcommand, Debug)]
pub enum FormulaCommands {
    /// Create a new formula template
    New {
        /// Package name
        name: String,
        /// Directory to save the formula in
        #[arg(long, default_value = "formulas")]
        output_dir: PathBuf,
    },
    /// Validate a formula file
    Check {
        /// Formula file to check
        path: PathBuf,
    },
    /// Bump a formula version
    Bump {
        /// Formula file to bump
        path: PathBuf,
        /// New version
        #[arg(long)]
        version: String,
        /// New bottle URL for current arch
        #[arg(long)]
        url: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let dry_run = cli.dry_run;

    match cli.command {
        Commands::Install { packages, locked } => {
            cmd::install::install(&packages, dry_run, locked).await
        }
        Commands::Remove { packages } => {
            cmd::remove::remove(&packages, dry_run)
        }
        Commands::List => {
            cmd::list::list()
        }
        Commands::Info { package } => {
            cmd::info::info(&package)
        }
        Commands::Hash { files } => {
            cmd::hash::hash(&files)
        }
        Commands::Lock => {
            cmd::lock::lock(dry_run)
        }
        Commands::Search { query } => {
            cmd::search::search(&query)
        }
        Commands::GenerateIndex { formulas_dir, output } => {
            generate_index(&formulas_dir, &output)
        }
        Commands::Clean => {
            cmd::clean::clean(dry_run)
        }
        Commands::Update { url } => {
            cmd::update::update(&url, dry_run).await
        }
        Commands::Upgrade { packages } => {
            cmd::upgrade::upgrade(&packages, dry_run).await
        }
        Commands::Formula { command } => {
            match command {
                FormulaCommands::New { name, output_dir } => {
                    cmd::formula::new(&name, &output_dir)
                }
                FormulaCommands::Check { path } => {
                    cmd::formula::check(&path)
                }
                FormulaCommands::Bump { path, version, url } => {
                    cmd::formula::bump(&path, &version, &url).await
                }
            }
        }
        Commands::Completions { shell } => {
            cmd::completions::completions(shell);
            Ok(())
        }
        Commands::SelfUpdate => {
            cmd::self_update::self_update(dry_run).await
        }
        Commands::Run { package, args } => {
            println!("🚀 Preparing to run '{}'...", package);
            cmd::run::run(&package, &args, dry_run).await
        }
    }
}

/// Generate index.bin from formulas directory
fn generate_index(formulas_dir: &std::path::Path, output: &std::path::Path) -> Result<()> {
    use dl::index::{PackageIndex, IndexEntry, IndexBottle};
    use std::time::{SystemTime, UNIX_EPOCH};
    
    let mut index = PackageIndex::new();
    index.updated_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    
    // Read all formula files
    for entry in std::fs::read_dir(formulas_dir)? {
        let entry = entry?;
        let path = entry.path();
        
        if path.extension().is_some_and(|ext| ext == "toml") {
            let formula = dl::formula::Formula::from_file(&path)?;
            
            let bottles: Vec<IndexBottle> = formula.bottle.iter()
                .map(|(arch, bottle)| IndexBottle {
                    arch: arch.clone(),
                    url: bottle.url.clone(),
                    blake3: bottle.blake3.clone(),
                })
                .collect();
            
            let hints_str = formula.hints.post_install.clone();
            
            index.upsert(IndexEntry {
                name: formula.package.name.clone(),
                version: formula.package.version.clone(),
                description: formula.package.description.clone(),
                deps: formula.dependencies.runtime.clone(),
                bottles,
                bin: formula.install.bin.clone(),
                hints: hints_str,
            });
            
            println!("  + {}", formula.package.name);
        }
    }
    
    index.save(output)?;
    println!("✓ Generated {} with {} packages", output.display(), index.packages.len());
    
    Ok(())
}
