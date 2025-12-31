//! apl-pkg - Unified registry management tool
//! Usage: cargo run --bin apl-pkg -- <command> [args]

use anyhow::{Context, Result};
use apl::registry::{build_github_client, github};
use clap::{Parser, Subcommand};
use std::fs;
use std::path::Path;

#[derive(Parser)]
#[command(name = "apl-pkg")]
#[command(about = "Unified APL package registry maintainer", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Add one or more packages from GitHub
    Add {
        /// GitHub repositories in owner/repo format
        repos: Vec<String>,
    },
    /// Update all existing packages or a specific one
    Update {
        /// Optional specific package to update
        #[arg(short, long)]
        package: Option<String>,
    },
    /// Lint and validate all package definitions
    Check,
    /// Regenerate the index.bin
    Index,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let token = std::env::var("GITHUB_TOKEN").ok();
    let client = build_github_client(token.as_deref())?;

    let packages_dir = std::env::current_dir()?.join("packages");
    let index_path = std::env::current_dir()?.join("index.bin");

    match cli.command {
        Commands::Add { repos } => {
            for repo in repos {
                println!("Adding {}...", repo);
                if let Err(e) = add_package(&client, &repo, &packages_dir).await {
                    eprintln!("   Failed: {}", e);
                }
            }
        }
        Commands::Update { package } => {
            println!("Updating packages...");
            let mut updated_count = 0;

            for entry in fs::read_dir(&packages_dir)? {
                let entry = entry?;
                let path = entry.path();

                if path.extension().is_some_and(|e| e == "toml") {
                    let file_name = path.file_stem().unwrap().to_string_lossy();
                    if let Some(ref target) = package {
                        if file_name != *target {
                            continue;
                        }
                    }

                    match github::update_package_definition(&client, &path).await {
                        Ok(updated) => {
                            if updated {
                                updated_count += 1;
                            }
                        }
                        Err(e) => eprintln!("   Failed to update {}: {}", file_name, e),
                    }
                }
            }

            if updated_count > 0 {
                cli_index(&packages_dir, &index_path)?;
                println!("\nDone! Updated {} packages.", updated_count);
            } else {
                println!("All packages up to date.");
            }
        }
        Commands::Check => {
            println!("Validating registry integrity...");
            let mut errors = 0;
            for entry in fs::read_dir(&packages_dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "toml") {
                    let content = fs::read_to_string(&path)?;
                    let pkg: serde_json::Value = toml::from_str(&content)?; // Just check basic structure
                    let name = pkg["package"]["name"].as_str().unwrap_or("unknown");
                    let version = pkg["package"]["version"].as_str().unwrap_or("");

                    if version == "0.0.0" || version.is_empty() {
                        eprintln!("   {}: Invalid version '{}'", name, version);
                        errors += 1;
                    }
                }
            }
            if errors == 0 {
                println!("   All packages valid.");
            } else {
                anyhow::bail!("Registry check failed with {} errors.", errors);
            }
        }
        Commands::Index => {
            cli_index(&packages_dir, &index_path)?;
        }
    }

    Ok(())
}

fn cli_index(packages_dir: &Path, index_path: &Path) -> Result<()> {
    println!("Regenerating index...");
    let index = apl::index::PackageIndex::generate_from_dir(packages_dir)?;
    index.save_compressed(index_path)?;
    println!("   Done: {}", index_path.display());
    Ok(())
}

async fn add_package(client: &reqwest::Client, repo: &str, out_dir: &Path) -> Result<()> {
    let parts: Vec<&str> = repo.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid repo format. Use owner/repo (e.g., jqlang/jq)");
    }
    let owner = parts[0];
    let repo_name = parts[1];

    let release = github::fetch_latest_release(client, owner, repo_name).await?;
    let version = github::strip_tag_prefix(&release.tag_name, repo_name);

    let (asset, is_archive) =
        github::find_best_asset(&release).context("No compatible macOS ARM64 asset found")?;

    println!("   Found asset: {}", asset.name);
    println!("   Downloading...");
    let bytes = client
        .get(&asset.browser_download_url)
        .send()
        .await?
        .bytes()
        .await?;
    let hash = blake3::hash(&bytes).to_hex().to_string();
    println!("   BLAKE3: {}", hash);

    let strip_components = if is_archive { 1 } else { 0 };

    // Determine format from asset name
    let format = if asset.name.ends_with(".tar.gz") {
        "tar.gz"
    } else if asset.name.ends_with(".tar.zst") || asset.name.ends_with(".tzst") {
        "tar.zst"
    } else if asset.name.ends_with(".tar.xz") || asset.name.ends_with(".tar") {
        "tar"
    } else if asset.name.ends_with(".zip") {
        "zip"
    } else if asset.name.ends_with(".dmg") {
        "dmg"
    } else if asset.name.ends_with(".pkg") {
        "pkg"
    } else {
        "binary"
    };

    let toml_content = format!(
        r#"[package]
name = "{}"
version = "{}"
description = ""
homepage = "https://github.com/{}"
license = ""
type = "cli"

[source]
url = "{}"
blake3 = "{}"
format = "{}"
strip_components = {}

[binary.arm64]
url = "{}"
blake3 = "{}"
format = "{}"

[dependencies]
runtime = []
build = []
optional = []

[install]
bin = ["{}"]

[hints]
post_install = ""
"#,
        repo_name,
        version,
        repo,
        asset.browser_download_url,
        hash,
        format,
        strip_components,
        asset.browser_download_url,
        hash,
        format,
        repo_name
    );

    let toml_path = out_dir.join(format!("{}.toml", repo_name));
    fs::write(&toml_path, toml_content)?;
    println!("   Created {}", toml_path.display());

    Ok(())
}
