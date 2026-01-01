//! apl-pkg - Unified registry management tool
//! Usage: cargo run --bin apl-pkg -- <command> [args]

use anyhow::{Context, Result};
use apl::arch::ARM64;
use apl::index::{IndexBinary, IndexSource, PackageIndex, VersionInfo};
use apl::package::{
    ArtifactFormat, Binary, Dependencies, Hints, InstallSpec, Package, PackageInfo, PackageType,
    Source,
};
use apl::registry::{build_github_client, github};
use clap::{Parser, Subcommand};
use std::collections::HashMap;
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
    /// Synchronize all existing packages or a specific one
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
            println!("Syncing packages...");
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
                    // Parse into full Package struct to validate schema
                    match Package::parse(&content) {
                        Ok(pkg) => {
                            if pkg.package.version == "0.0.0" || pkg.package.version.is_empty() {
                                eprintln!(
                                    "   {}: Invalid version '{}'",
                                    pkg.package.name, pkg.package.version
                                );
                                errors += 1;
                            }
                        }
                        Err(e) => {
                            eprintln!("   {}: Invalid TOML structure: {}", path.display(), e);
                            errors += 1;
                        }
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
    let index = generate_index_from_dir(packages_dir)?;
    index.save_compressed(index_path)?;
    println!("   Done: {}", index_path.display());
    Ok(())
}

fn generate_index_from_dir(dir: &Path) -> Result<PackageIndex> {
    let mut index = PackageIndex::new();
    index.updated_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().is_some_and(|ext| ext == "toml") {
            let pkg = Package::from_file(&path)
                .map_err(|e| anyhow::anyhow!("Failed to parse {}: {}", path.display(), e))?;

            // VALIDATION: Fail fast on invalid versions
            if pkg.package.version.is_empty() || pkg.package.version == "0.0.0" {
                anyhow::bail!(
                    "Package '{}' has invalid version '{}'. This indicates the package was not properly populated. Fix the package or remove it before generating the index.",
                    pkg.package.name,
                    pkg.package.version
                );
            }

            let binaries: Vec<IndexBinary> = pkg
                .binary
                .iter()
                .map(|(arch, binary)| IndexBinary {
                    arch: arch.clone(),
                    url: binary.url.clone(),
                    blake3: binary.blake3.clone(),
                })
                .collect();

            let release = VersionInfo {
                version: pkg.package.version.clone(),
                binaries,
                deps: pkg.dependencies.runtime.clone(),
                build_deps: pkg.dependencies.build.clone(),
                build_script: pkg
                    .build
                    .as_ref()
                    .map(|b| b.script.clone())
                    .unwrap_or_default(),
                bin: pkg.install.bin.clone(),
                hints: pkg.hints.post_install.clone(),
                app: pkg.install.app.clone(),
                source: Some(IndexSource {
                    url: pkg.source.url.clone(),
                    blake3: pkg.source.blake3.clone(),
                }),
            };

            let type_str = match pkg.package.type_ {
                PackageType::Cli => "cli",
                PackageType::App => "app",
            };

            index.upsert_release(
                &pkg.package.name,
                &pkg.package.description,
                type_str,
                release,
            );
        }
    }
    Ok(index)
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

    let (asset, is_archive) = github::find_best_asset(&release, repo_name)
        .context("No compatible macOS ARM64 asset found")?;

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
        ArtifactFormat::TarGz
    } else if asset.name.ends_with(".tar.zst") || asset.name.ends_with(".tzst") {
        ArtifactFormat::TarZst
    } else if asset.name.ends_with(".tar.xz") || asset.name.ends_with(".tar") {
        ArtifactFormat::Tar
    } else if asset.name.ends_with(".zip") {
        ArtifactFormat::Zip
    } else if asset.name.ends_with(".dmg") {
        ArtifactFormat::Dmg
    } else if asset.name.ends_with(".pkg") {
        ArtifactFormat::Pkg
    } else {
        ArtifactFormat::Binary
    };

    let mut binary_map = HashMap::new();
    // Assuming ARM64 for now as default, maybe detect based on asset name in future if needed
    binary_map.insert(
        ARM64.to_string(),
        Binary {
            url: asset.browser_download_url.clone(),
            blake3: hash.clone(),
            format: format.clone(),
            arch: ARM64.to_string(),
            macos: "14.0".to_string(),
        },
    );

    let package = Package {
        package: PackageInfo {
            name: repo_name.to_string(),
            version: version.to_string(),
            description: "".to_string(),
            homepage: format!("https://github.com/{}", repo),
            license: "".to_string(),
            type_: PackageType::Cli,
        },
        source: Source {
            url: asset.browser_download_url.clone(),
            blake3: hash.clone(),
            format: format.clone(),
            strip_components: strip_components,
        },
        binary: binary_map,
        dependencies: Dependencies::default(),
        install: InstallSpec {
            bin: vec![repo_name.to_string()],
            ..Default::default()
        },
        hints: Hints {
            post_install: "".to_string(),
        },
        build: None,
    };

    let toml_content = package.to_toml()?;

    let toml_path = out_dir.join(format!("{}.toml", repo_name));
    fs::write(&toml_path, toml_content)?;
    println!("   Created {}", toml_path.display());

    Ok(())
}
