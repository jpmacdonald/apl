use anyhow::Result;
use apl::indexer::sources::github::{self, build_client};
use apl::package::Package;
use clap::{Parser, Subcommand};
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
        /// Force full rebuild (ignore existing index)
        #[arg(long)]
        full: bool,
    },
    /// Migrate legacy registry to Selectors Pattern
    MigrateSelectors,
    /// Generate a new Ed25519 signing keypair
    Keygen,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let token = std::env::var("GITHUB_TOKEN").ok();
    let client = build_client(token.as_deref())?;

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
                cli_index(
                    &client,
                    &registry_dir,
                    &index_path,
                    package.as_deref(),
                    false,
                )
                .await?;
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
                // Try parsing as PackageTemplate first (algorithmic registry)
                match apl::package::PackageTemplate::parse(&content) {
                    Ok(_) => {}
                    Err(e1) => {
                        // Fallback to legacy Package parse
                        match Package::parse(&content) {
                            Ok(pkg) => {
                                if pkg.package.version == "0.0.0" || pkg.package.version.is_empty()
                                {
                                    eprintln!(
                                        "   {}: Invalid version '{}'",
                                        pkg.package.name, pkg.package.version
                                    );
                                    errors += 1;
                                }
                            }
                            Err(e2) => {
                                eprintln!("   {}: Invalid TOML structure:", path.display());
                                eprintln!("      As Template: {e1}");
                                eprintln!("      As Legacy:   {e2}");
                                errors += 1;
                            }
                        }
                    }
                }
            }
            if errors == 0 {
                println!("   All packages valid.");
            } else {
                anyhow::bail!("Registry check failed with {errors} errors.");
            }
        }
        Commands::Index { package, full } => {
            cli_index(
                &client,
                &registry_dir,
                &index_path,
                package.as_deref(),
                full,
            )
            .await?;
        }
        Commands::MigrateSelectors => {
            cli_migrate_selectors(&registry_dir).await?;
        }
        Commands::Keygen => {
            cli_keygen()?;
        }
    }

    Ok(())
}

// HashCache logic moved to src/indexer/hashing.rs

// fetch_and_parse_checksum moved to src/indexer/hashing.rs

// get_github_asset_digest moved to src/indexer/discovery.rs

// discover_versions moved to src/indexer/discovery.rs

// extract_version_from_tag moved to src/indexer/discovery.rs

async fn cli_migrate_selectors(_registry_dir: &Path) -> Result<()> {
    // Migration is complete, this function can be simplified or removed later.
    // For now, I'll leave a skeleton that returns Ok.
    Ok(())
}

async fn cli_index(
    client: &reqwest::Client,
    registry_dir: &Path,
    index_path: &Path,
    package_filter: Option<&str>,
    force_full: bool,
) -> Result<()> {
    println!("Regenerating index...");

    let index = apl::indexer::generate_index_from_registry(
        client,
        registry_dir,
        package_filter,
        force_full,
    )
    .await?;

    index.save_compressed(index_path)?;

    // Sign the index if key is present
    if let Ok(secret_b64) = std::env::var("APL_SIGNING_KEY") {
        use base64::Engine;
        use ed25519_dalek::{Signer, SigningKey};

        println!("🔒 Signing index...");
        let secret_bytes = base64::engine::general_purpose::STANDARD
            .decode(secret_b64.trim())
            .expect("Invalid Base64 signing key");

        let signing_key = SigningKey::from_bytes(
            secret_bytes
                .as_slice()
                .try_into()
                .expect("Key must be 32 bytes"),
        );

        // Read back the exact file we just wrote to ensure signature matches disk content
        let index_data = fs::read(index_path)?;
        let signature = signing_key.sign(&index_data);

        let sig_b64 = base64::engine::general_purpose::STANDARD.encode(signature.to_bytes());
        let sig_path = index_path.with_extension("bin.sig");
        fs::write(&sig_path, sig_b64)?;
        println!("   Created signature: {}", sig_path.display());
    } else {
        println!("⚠️  APL_SIGNING_KEY not set. Index is UNSIGNED.");
    }

    // Export latest 'apl' info for the Cloudflare Worker router
    if let Some(entry) = index.find("apl") {
        if let Some(latest) = entry.latest() {
            let mut content = format!("version={}\n", latest.version);
            for bin in &latest.binaries {
                let key = match bin.arch {
                    apl::types::Arch::Arm64 => "darwin_arm64",
                    apl::types::Arch::X86_64 => "darwin_x86_64",
                    apl::types::Arch::Universal => "darwin_universal",
                };
                content.push_str(&format!("{key}={}\n", bin.url));
            }

            let info_path = index_path.with_file_name("latest.txt");
            fs::write(&info_path, content)?;
            println!("   Generated manifest: {}", info_path.display());
        }
    }

    Ok(())
}
async fn add_package(client: &reqwest::Client, repo: &str, out_dir: &Path) -> Result<()> {
    use apl::package::{
        AssetConfig, AssetSelector, DiscoveryConfig, Hints, InstallSpec, PackageInfoTemplate,
        PackageTemplate,
    };
    use apl::types::PackageName;

    let parts: Vec<&str> = repo.split('/').collect();
    if parts.len() != 2 {
        anyhow::bail!("Invalid repo format. Use owner/repo (e.g., jqlang/jq)");
    }
    let owner = parts[0];
    let repo_name = parts[1];

    let release = github::fetch_latest_release(client, owner, repo_name).await?;

    // Guess tag pattern
    let tag_pattern = if release.tag_name.starts_with('v') {
        "v{{version}}".to_string()
    } else {
        "{{version}}".to_string()
    };

    // Find macOS assets
    let (arm64_asset, x86_asset) = github::find_macos_assets(&release, repo_name);

    let mut select = HashMap::new();

    if let Some(asset) = arm64_asset {
        // Simple heuristic: take suffix after version or just the whole thing
        select.insert(
            "arm64-macos".to_string(),
            AssetSelector::Suffix {
                suffix: asset
                    .name
                    .split('-')
                    .next_back()
                    .unwrap_or(&asset.name)
                    .to_string(),
            },
        );
    }

    if let Some(asset) = x86_asset {
        select.insert(
            "x86_64-macos".to_string(),
            AssetSelector::Suffix {
                suffix: asset
                    .name
                    .split('-')
                    .next_back()
                    .unwrap_or(&asset.name)
                    .to_string(),
            },
        );
    }

    let template = PackageTemplate {
        package: PackageInfoTemplate {
            name: PackageName::from(repo_name.to_string()),
            description: String::new(), // Fetching from GitHub API would be better
            homepage: format!("https://github.com/{repo}"),
            license: String::new(),
            tags: vec![],
        },
        discovery: DiscoveryConfig::GitHub {
            github: repo.to_string(),
            tag_pattern,
            include_prereleases: false,
        },
        assets: AssetConfig {
            universal: false,
            select,
            skip_checksums: false,
            checksum_url: None,
        },
        source: None,
        build: None,
        install: InstallSpec::default(),
        hints: Hints::default(),
    };

    let target_path = apl::indexer::registry_path(out_dir, repo_name);
    fs::create_dir_all(target_path.parent().unwrap())?;
    let template_toml = toml::to_string_pretty(&template)?;
    fs::write(&target_path, template_toml)?;

    println!("   Created template: {}", target_path.display());
    Ok(())
}

fn cli_keygen() -> Result<()> {
    use base64::Engine;
    use ed25519_dalek::SigningKey;

    use std::io::Write;

    use rand::RngCore;

    println!("🔑 Generating new Ed25519 signing keypair...");

    let mut secret_bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut secret_bytes);
    let signing_key = SigningKey::from_bytes(&secret_bytes);
    let verify_key = signing_key.verifying_key();

    // Encode as Base64 (Standard engine)
    let secret_b64 = base64::engine::general_purpose::STANDARD.encode(signing_key.to_bytes());
    let public_b64 = base64::engine::general_purpose::STANDARD.encode(verify_key.to_bytes());

    println!("\n{:=^60}", " SECRET KEY (Keep this safe!) ");
    println!("{secret_b64}");
    println!("{:=^60}\n", "");

    println!("{:=^60}", " PUBLIC KEY (Embed in app) ");
    println!("{public_b64}");
    println!("{:=^60}\n", "");

    // Save to keyfile for convenience
    let keyfile_path = Path::new("apl.key");
    if !keyfile_path.exists() {
        let mut f = fs::File::create(keyfile_path)?;
        f.write_all(secret_b64.as_bytes())?;
        println!("✓ Secret key saved to ./apl.key (gitignore this!)");
    }

    Ok(())
}
