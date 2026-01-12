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
        /// Show verbose output (DMG mounting, file counts, etc.)
        #[arg(short, long)]
        verbose: bool,
    },
    /// Remove a package
    Remove {
        /// Package name(s)
        #[arg(required_unless_present = "all")]
        packages: Vec<String>,
        /// Remove all installed packages
        #[arg(long, short = 'a', conflicts_with = "packages")]
        all: bool,
        /// Skip confirmation prompt
        #[arg(long, short = 'y')]
        yes: bool,
        /// Force removal of package metadata even if files are missing
        #[arg(long, short = 'f')]
        force: bool,
    },
    /// Switch active version of a package
    Use {
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
    /// Compute SHA256 hash of a file (for package authoring)
    #[command(hide = true)]
    Hash {
        /// Files to hash
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    /// Search available packages
    Search {
        /// Search query
        query: String,
    },
    /// Remove orphaned CAS blobs and temp files
    Clean,
    /// Update package index from CDN
    Update {
        /// CDN URL for index
        #[arg(long, env = "APL_INDEX_URL", default_value = "https://apl.pub/index")]
        url: String,
    },
    /// Upgrade installed packages to latest versions
    Upgrade {
        /// Specific packages to upgrade (or all if empty)
        packages: Vec<String>,
        /// Skip confirmation prompt
        #[arg(short, long)]
        yes: bool,
    },
    /// Check status of installed packages
    Status,
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
    #[command(name = "self-update")]
    SelfUpdate,
    /// Run a package without installing it globally
    Run {
        /// Package name
        package: String,
        /// Arguments for the package
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Enter a project-scoped shell environment
    Shell {
        /// Fail if lockfile is missing or out of sync (for CI)
        #[arg(long)]
        frozen: bool,
        /// Force re-resolution even if lockfile is valid
        #[arg(long)]
        update: bool,
        /// Optional command to run inside the shell
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Option<Vec<String>>,
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
        /// Package file to check
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

    // Auto-detect `apl shell` context
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 1 {
        // No arguments provided. Check for apl.toml
        if let Ok(cwd) = std::env::current_dir() {
            if has_manifest(&cwd) {
                println!("🔮 Found apl.toml. Entering project shell...");
                return cmd::shell::shell(false, false, None).await;
            }
        }
    }

    let cli = Cli::parse();
    let dry_run = cli.dry_run;

    match cli.command {
        Commands::Install { packages, verbose } => {
            cmd::install::install(&packages, dry_run, verbose).await
        }
        Commands::Remove {
            packages,
            all,
            yes,
            force,
        } => cmd::remove::remove(&packages, all, yes, force, dry_run).await,
        Commands::Use { spec } => cmd::r#use::use_package(&spec, dry_run),
        Commands::History { package } => cmd::history::history(&package),
        Commands::Rollback { package } => cmd::rollback::rollback(&package, dry_run).await,
        Commands::List => cmd::list::list(),
        Commands::Info { package } => cmd::info::info(&package),
        Commands::Hash { files } => cmd::hash::hash(&files),
        Commands::Search { query } => cmd::search::search(&query),
        Commands::Clean => cmd::clean::clean(dry_run),
        Commands::Update { url } => cmd::update::update(&url, dry_run).await,
        Commands::Upgrade { packages, yes } => cmd::upgrade::upgrade(&packages, yes, dry_run).await,

        Commands::Status => cmd::status::status(),
        Commands::Package { command } => match command {
            PackageCommands::New { name, output_dir } => cmd::package::new(&name, &output_dir),
            PackageCommands::Check { path } => cmd::package::check(&path),
            PackageCommands::Bump { path, version, url } => {
                cmd::package::bump(&path, &version, &url).await
            }
        },
        Commands::Completions { shell } => {
            cmd::completions::completions(shell);
            Ok(())
        }
        Commands::SelfUpdate => cmd::self_update::self_update(dry_run).await,
        Commands::Run { package, args } => {
            println!("Preparing to run '{package}'...");
            cmd::run::run(&package, &args, dry_run).await
        }
        Commands::Shell {
            frozen,
            update,
            command,
        } => cmd::shell::shell(frozen, update, command).await,
    }
}

fn has_manifest(start: &std::path::Path) -> bool {
    let mut current = start;
    loop {
        if current.join("apl.toml").exists() {
            return true;
        }
        if let Some(parent) = current.parent() {
            current = parent;
        } else {
            return false;
        }
    }
}
