//! apl-pkg - Unified registry management tool
//! Usage: cargo run --bin apl-pkg -- <command> [args]

use anyhow::Result;
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

            struct UpdateResult {
                name: String,
                status: UpdateStatus,
            }

            enum UpdateStatus {
                Updated,
                UpToDate,
                Failed(String),
            }

            let mut results = Vec::new();

            for entry in fs::read_dir(&packages_dir)? {
                let entry = entry?;
                let path = entry.path();

                if path.extension().is_some_and(|e| e == "toml") {
                    let file_name = path.file_stem().unwrap().to_string_lossy().to_string();
                    if let Some(ref target) = package {
                        if file_name != *target {
                            continue;
                        }
                    }

                    match github::update_package_definition(&client, &path).await {
                        Ok(updated) => {
                            if updated {
                                results.push(UpdateResult {
                                    name: file_name,
                                    status: UpdateStatus::Updated,
                                });
                            } else {
                                results.push(UpdateResult {
                                    name: file_name,
                                    status: UpdateStatus::UpToDate,
                                });
                            }
                        }
                        Err(e) => {
                            eprintln!("   Failed to update {}: {}", file_name, e); // Keep inline error for context
                            results.push(UpdateResult {
                                name: file_name,
                                status: UpdateStatus::Failed(e.to_string()),
                            });
                        }
                    }
                }
            }

            // Calculate stats
            let updated_count = results
                .iter()
                .filter(|r| matches!(r.status, UpdateStatus::Updated))
                .count();
            let failed_count = results
                .iter()
                .filter(|r| matches!(r.status, UpdateStatus::Failed(_)))
                .count();

            if updated_count > 0 {
                cli_index(&packages_dir, &index_path)?;
            }

            // Print Summary
            if failed_count > 0 || updated_count > 0 {
                println!("\n{:=^40}", " Update Summary ");

                if updated_count > 0 {
                    println!("\nUpdated ({})", updated_count);
                    for r in results
                        .iter()
                        .filter(|r| matches!(r.status, UpdateStatus::Updated))
                    {
                        println!("  ✓ {}", r.name);
                    }
                }

                if failed_count > 0 {
                    println!("\nFailed ({})", failed_count);
                    for r in results
                        .iter()
                        .filter(|r| matches!(r.status, UpdateStatus::Failed(_)))
                    {
                        if let UpdateStatus::Failed(msg) = &r.status {
                            println!("  ✗ {}: {}", r.name, msg);
                        }
                    }
                }
                println!("\n{:=^40}\n", "");
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

    // NEW: Find BOTH ARM64 and x86_64 assets
    let (arm64_asset, x86_asset) = github::find_macos_assets(&release, repo_name);

    if arm64_asset.is_none() && x86_asset.is_none() {
        anyhow::bail!("No compatible macOS assets found (neither ARM64 nor x86_64)");
    }

    // Helper to download and hash an asset
    async fn download_asset(
        client: &reqwest::Client,
        asset: &github::GithubAsset,
    ) -> Result<(String, String)> {
        let bytes = client
            .get(&asset.browser_download_url)
            .send()
            .await?
            .bytes()
            .await?;
        let hash = blake3::hash(&bytes).to_hex().to_string();
        Ok((asset.browser_download_url.clone(), hash))
    }

    // Determine format from first available asset
    let sample_asset = arm64_asset.or(x86_asset).unwrap();
    let is_archive = sample_asset.name.ends_with(".tar.gz")
        || sample_asset.name.ends_with(".zip")
        || sample_asset.name.ends_with(".tar.xz")
        || sample_asset.name.ends_with(".tar.zst")
        || sample_asset.name.ends_with(".tzst")
        || sample_asset.name.ends_with(".dmg")
        || sample_asset.name.ends_with(".pkg");

    let strip_components = if is_archive { 1 } else { 0 };

    let format = if sample_asset.name.ends_with(".tar.gz") {
        ArtifactFormat::TarGz
    } else if sample_asset.name.ends_with(".tar.zst") || sample_asset.name.ends_with(".tzst") {
        ArtifactFormat::TarZst
    } else if sample_asset.name.ends_with(".tar.xz") || sample_asset.name.ends_with(".tar") {
        ArtifactFormat::Tar
    } else if sample_asset.name.ends_with(".zip") {
        ArtifactFormat::Zip
    } else if sample_asset.name.ends_with(".dmg") {
        ArtifactFormat::Dmg
    } else if sample_asset.name.ends_with(".pkg") {
        ArtifactFormat::Pkg
    } else {
        ArtifactFormat::Binary
    };

    // Download ARM64
    let mut binary_map = HashMap::new();
    let source_url;
    let source_hash;

    if let Some(asset) = arm64_asset {
        println!("   Found ARM64 asset: {}", asset.name);
        println!("   Downloading...");
        let (url, hash) = download_asset(client, asset).await?;
        println!("   ARM64 BLAKE3: {}", hash);

        binary_map.insert(
            "arm64".to_string(),
            Binary {
                url: url.clone(),
                blake3: hash.clone(),
                format: format.clone(),
                arch: "arm64".to_string(),
                macos: "14.0".to_string(),
            },
        );

        // Use ARM64 as source by default
        source_url = url;
        source_hash = hash;
    } else {
        // Use x86_64 as source if no ARM64
        source_url = String::new();
        source_hash = String::new();
    }

    // Download x86_64
    if let Some(asset) = x86_asset {
        println!("   Found x86_64 asset: {}", asset.name);
        println!("   Downloading...");
        let (url, hash) = download_asset(client, asset).await?;
        println!("   x86_64 BLAKE3: {}", hash);

        binary_map.insert(
            "x86_64".to_string(),
            Binary {
                url: url.clone(),
                blake3: hash.clone(),
                format: format.clone(),
                arch: "x86_64".to_string(),
                macos: "14.0".to_string(),
            },
        );

        // If we didn't have ARM64, use x86_64 as source
        if arm64_asset.is_none() {
            let source_url = url;
            let source_hash = hash;

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
                    url: source_url,
                    blake3: source_hash,
                    format: format.clone(),
                    strip_components,
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
            return Ok(());
        }
    }

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
            url: source_url,
            blake3: source_hash,
            format: format.clone(),
            strip_components,
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
