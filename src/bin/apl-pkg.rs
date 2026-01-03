use anyhow::Result;
use apl::package::{
    ArtifactFormat, Binary, Dependencies, Hints, InstallSpec, Package, PackageInfo, PackageType,
    Source,
};
use apl::registry::{build_github_client, github};
use apl::types::{Arch, PackageName, Version};
use clap::{Parser, Subcommand};
use sha2::Digest;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Parser)]
#[command(name = "apl-pkg")]
#[command(about = "Unified APL package registry maintainer", long_about = None)]
struct Cli {
    #[arg(short, long, default_value = "registry")]
    registry: std::path::PathBuf,

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
    Index {
        /// Optional specific package to index
        #[arg(short, long)]
        package: Option<String>,
    },
    /// Migrate legacy packages to algorithmic registry templates
    Migrate,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let token = std::env::var("GITHUB_TOKEN").ok();
    let client = build_github_client(token.as_deref())?;

    let registry_dir = cli.registry;
    let index_path = std::env::current_dir()?.join("index.bin");

    match cli.command {
        Commands::Add { repos } => {
            for repo in repos {
                println!("Adding {repo}...");
                if let Err(e) = add_package(&client, &repo, &registry_dir).await {
                    eprintln!("   Failed: {e}");
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

            // Walk registry (sharded or flat)
            let toml_files = apl::indexer::walk_registry_toml_files(&registry_dir)?;

            for path in toml_files {
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
                        eprintln!("   Failed to update {file_name}: {e}");
                        results.push(UpdateResult {
                            name: file_name,
                            status: UpdateStatus::Failed(e.to_string()),
                        });
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
                cli_index(&client, &registry_dir, &index_path, package.as_deref()).await?;
            }

            // Print Summary
            if failed_count > 0 || updated_count > 0 {
                println!("\n{:=^40}", " Update Summary ");

                if updated_count > 0 {
                    println!("\nUpdated ({updated_count})");
                    for r in results
                        .iter()
                        .filter(|r| matches!(r.status, UpdateStatus::Updated))
                    {
                        println!("  ✓ {}", r.name);
                    }
                }

                if failed_count > 0 {
                    println!("\nFailed ({failed_count})");
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

            let toml_files = apl::indexer::walk_registry_toml_files(&registry_dir)?;

            for path in toml_files {
                let content = fs::read_to_string(&path)?;
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
            if errors == 0 {
                println!("   All packages valid.");
            } else {
                anyhow::bail!("Registry check failed with {errors} errors.");
            }
        }
        Commands::Index { package } => {
            cli_index(&client, &registry_dir, &index_path, package.as_deref()).await?;
        }
        Commands::Migrate => {
            cli_migrate(&registry_dir, &registry_dir).await?;
        }
    }

    Ok(())
}

// HashCache logic moved to src/indexer/hashing.rs

// fetch_and_parse_checksum moved to src/indexer/hashing.rs

// get_github_asset_digest moved to src/indexer/discovery.rs

// discover_versions moved to src/indexer/discovery.rs

// extract_version_from_tag moved to src/indexer/discovery.rs

async fn cli_index(
    client: &reqwest::Client,
    registry_dir: &Path,
    index_path: &Path,
    package_filter: Option<&str>,
) -> Result<()> {
    println!("Regenerating index...");

    // Check if we should use new registry/ or old packages/
    let index = if registry_dir.exists() && registry_dir.is_dir() {
        println!("   Using algorithmic registry (registry/)...");
        apl::indexer::generate_index_from_registry(client, registry_dir, package_filter).await?
    } else {
        println!("   Using legacy packages directory...");
        apl::indexer::generate_index_from_dir(client, registry_dir, package_filter).await?
    };

    index.save_compressed(index_path)?;
    println!("   Done: {}", index_path.display());
    Ok(())
}

async fn cli_migrate(packages_dir: &Path, registry_dir: &Path) -> Result<()> {
    use apl::package::{AssetConfig, ChecksumConfig, DiscoveryConfig, Package, PackageTemplate};

    println!("Migrating packages to algorithmic registry...");

    if !packages_dir.exists() {
        anyhow::bail!(
            "Legacy packages directory not found: {}",
            packages_dir.display()
        );
    }

    fs::create_dir_all(registry_dir)?;

    let mut count = 0;
    for entry in fs::read_dir(packages_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().is_some_and(|e| e == "toml") {
            let name = path.file_stem().unwrap().to_string_lossy().to_string();
            println!("   Migrating {name}...");

            let toml_str = fs::read_to_string(&path)?;
            let pkg: Package = match toml::from_str(&toml_str) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("     ⚠ Failed to parse {name}: {e}");
                    continue;
                }
            };

            // Guess GitHub repo from source URL
            let repo = apl::indexer::guess_github_repo(&pkg.source.url);

            if let Some(repo) = repo {
                // Guess tag pattern (check if current version in TOML has 'v' prefix in source URL)
                let tag_pattern = if pkg
                    .source
                    .url
                    .contains(&format!("v{}", pkg.package.version))
                {
                    "v{{version}}".to_string()
                } else {
                    "{{version}}".to_string()
                };

                // Construct template
                let template = PackageTemplate {
                    package: pkg.package.clone(),
                    discovery: DiscoveryConfig::GitHub {
                        github: repo.clone(),
                        tag_pattern,
                        semver_only: true,
                        include_prereleases: false,
                    },
                    assets: AssetConfig {
                        url_template: apl::indexer::guess_url_template(
                            &pkg.source.url,
                            pkg.package.version.as_str(),
                            &repo,
                        ),
                        targets: apl::indexer::guess_targets(&pkg),
                        universal: false, // Default
                    },
                    checksums: ChecksumConfig {
                        url_template: None, // Will need manual review or default
                        vendor_type: Some(apl::index::HashType::Sha256),
                        skip: false,
                    },
                    install: pkg.install.clone(),
                    hints: pkg.hints.clone(),
                };

                let target_path = apl::indexer::registry_path(registry_dir, &name);
                fs::create_dir_all(target_path.parent().unwrap())?;

                let template_toml = toml::to_string_pretty(&template)?;
                fs::write(target_path, template_toml)?;
                count += 1;
            } else {
                println!("     ⚠ Could not guess GitHub repo for {name}, skipping.");
            }
        }
    }

    println!("   Migrated {count} packages.");
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
        let hash = hex::encode(sha2::Sha256::digest(&bytes));
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
        println!("   ARM64 SHA256: {hash}");

        binary_map.insert(
            Arch::Arm64,
            Binary {
                url: url.clone(),
                sha256: hash.clone(),
                format: format.clone(),
                arch: Arch::Arm64,
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
        println!("   x86_64 SHA256: {hash}");

        binary_map.insert(
            Arch::X86_64,
            Binary {
                url: url.clone(),
                sha256: hash.clone(),
                format: format.clone(),
                arch: Arch::X86_64,
                macos: "14.0".to_string(),
            },
        );

        // If we didn't have ARM64, use x86_64 as source
        if arm64_asset.is_none() {
            let source_url = url;
            let source_hash = hash;

            let package = Package {
                package: PackageInfo {
                    name: PackageName::from(repo_name.to_string()),
                    version: Version::from(version.to_string()),
                    description: "".to_string(),
                    homepage: format!("https://github.com/{repo}"),
                    license: "".to_string(),
                    type_: PackageType::Cli,
                },
                source: Source {
                    url: source_url,
                    sha256: source_hash,
                    format: format.clone(),
                    strip_components,
                    url_template: None,
                    versions: None,
                },
                targets: binary_map,
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
            let toml_path = out_dir.join(format!("{repo_name}.toml"));
            fs::write(&toml_path, toml_content)?;
            println!("   Created {}", toml_path.display());
            return Ok(());
        }
    }

    let package = Package {
        package: PackageInfo {
            name: PackageName::from(repo_name.to_string()),
            version: Version::from(version.to_string()),
            description: "".to_string(),
            homepage: format!("https://github.com/{repo}"),
            license: "".to_string(),
            type_: PackageType::Cli,
        },
        source: Source {
            url: source_url,
            sha256: source_hash,
            format: format.clone(),
            strip_components,
            url_template: None,
            versions: None,
        },
        targets: binary_map,
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
    let toml_path = out_dir.join(format!("{repo_name}.toml"));
    fs::write(&toml_path, toml_content)?;
    println!("   Created {}", toml_path.display());

    Ok(())
}
