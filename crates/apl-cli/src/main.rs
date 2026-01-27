//! apl - A Package Layer CLI

use anyhow::Result;
use clap::Parser;
use tracing_subscriber::EnvFilter;

use apl_cli::cmd;
use apl_cli::{Cli, Commands, PackageCommands};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    // 1. Pre-process arguments to filter out any '#' comments
    // This allows copy-pasting commands with trailing comments.
    let args: Vec<String> = std::env::args()
        .take_while(|arg| !arg.starts_with('#'))
        .collect();

    // Auto-detect `apl shell` context
    if args.len() == 1 {
        // No arguments provided. Check for apl.toml
        if let Ok(cwd) = std::env::current_dir() {
            if has_manifest(&cwd) {
                println!("🔮 Found apl.toml. Entering project shell...");
                return cmd::shell::shell(false, false, None).await;
            }
        }
    }

    let cli = Cli::parse_from(args);
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
        Commands::Update { url, all } => cmd::update::update(&url, all, dry_run).await,
        Commands::Upgrade { packages, yes } => cmd::upgrade::upgrade(&packages, yes, dry_run).await,

        Commands::Status => cmd::status::status(),
        Commands::Package { command } => match command {
            PackageCommands::New { name, output_dir } => cmd::package::new(&name, &output_dir),
            PackageCommands::Check { path } => cmd::package::check(&path),
            PackageCommands::Bump { path, version, url } => {
                cmd::package::bump(&path, &version, &url)
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
