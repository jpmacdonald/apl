//! apl - A Package Layer CLI

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

mod cmd;

#[derive(Parser)]
#[command(name = "apl")]
#[command(author, version, about = "apl - A Package Layer for macOS")]
pub struct Cli {
    /// Show what would happen without making changes
    #[arg(long, global = true)]
    dry_run: bool,

    /// Suppress non-essential output
    #[arg(short, long, global = true)]
    quiet: bool,

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
        /// Only install packages pinned in apl.lock
        #[arg(long)]
        locked: bool,
    },
    /// Remove a package
    Remove {
        /// Package name(s)
        #[arg(required = true)]
        packages: Vec<String>,
    },
    /// Switch active version of a package
    Switch {
        /// Package spec (e.g. jq@1.6)
        spec: String,
    },
    /// View package history
    History {
        /// Package name
        package: String,
    },
    /// Rollback package to previous state
    Rollback {
        /// Package name
        package: String,
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
    /// Generate or update apl.lock from installed packages
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
        #[arg(long, env = "APL_INDEX_URL", default_value = "https://raw.githubusercontent.com/jpmacdonald/distill/gh-pages/index.bin")]
        url: String,
    },
    /// Upgrade installed packages to latest versions
    Upgrade {
        /// Specific packages to upgrade (or all if empty)
        packages: Vec<String>,
    },
    /// Package management commands
    Package {
        #[command(subcommand)]
        command: PackageCommands,
    },
    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        shell: clap_complete::Shell,
    },
    /// Update apl itself to the latest version
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
pub enum PackageCommands {
    /// Create a new package template
    New {
        /// Package name
        name: String,
        /// Directory to save the package in
        #[arg(long, default_value = "packages")]
        output_dir: PathBuf,
    },
    /// Validate a package file
    Check {
        /// Package file to check
        path: PathBuf,
    },
    /// Bump a package version
    Bump {
        /// Package file to bump
        path: PathBuf,
        /// New version
        #[arg(long)]
        version: String,
        /// New binary URL for current arch
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
        Commands::Switch { spec } => {
            cmd::switch::switch(&spec, dry_run)
        }
        Commands::History { package } => {
            cmd::history::history(&package)
        }
        Commands::Rollback { package } => {
            cmd::rollback::rollback(&package, dry_run)
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
            cmd::generate_index::generate_index(&formulas_dir, &output)
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
        Commands::Package { command } => {
            match command {
                PackageCommands::New { name, output_dir } => {
                    cmd::package::new(&name, &output_dir)
                }
                PackageCommands::Check { path } => {
                    cmd::package::check(&path)
                }
                PackageCommands::Bump { path, version, url } => {
                    cmd::package::bump(&path, &version, &url).await
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

