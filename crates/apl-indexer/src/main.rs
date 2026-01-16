use anyhow::{Context, Result};
use apl_core::indexer::forges::github::{self, build_client};
use apl_core::package::Package;
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
    /// Regenerate the index
    Index {
        /// Optional specific package to index
        #[arg(short, long)]
        package: Option<String>,
        /// Force full rebuild (ignore existing index)
        #[arg(long)]
        full: bool,
        /// Bootstrap index from URL (download before indexing if local index missing)
        #[arg(long)]
        bootstrap: Option<String>,
        /// Show detailed per-package progress
        #[arg(short, long)]
        verbose: bool,
        /// Output path for the index file
        #[arg(short, long)]
        output: Option<std::path::PathBuf>,
    },

    /// Generate a new Ed25519 signing keypair
    Keygen,
    /// Import package definitions from other repositories
    Import {
        /// Import source (e.g. "homebrew")
        #[arg(long, default_value = "homebrew")]
        from: String,
        /// Package names to import
        packages: Vec<String>,
    },
    /// Verify a package by installing and smoke-testing
    Verify {
        /// Path to the package TOML file
        package: std::path::PathBuf,
    },
    /// Sign an arbitrary file using APL_SIGNING_KEY
    Sign {
        /// Input file to sign
        #[arg(short, long)]
        input: std::path::PathBuf,
        /// Output signature file
        #[arg(short, long)]
        output: std::path::PathBuf,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let token = std::env::var("GITHUB_TOKEN").ok();
    let client = build_client(token.as_deref())?;

    let registry_dir = cli.registry;
    let index_path = std::env::current_dir()?.join("index");

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
            let toml_files = apl_core::indexer::walk_registry_toml_files(&registry_dir)?;

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
                    None,
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
            let mut warnings = 0;

            let toml_files: Vec<_> =
                apl_core::indexer::walk_registry_toml_files(&registry_dir)?.collect();
            let mut known_packages = std::collections::HashSet::new();
            let mut templates = Vec::new();

            // Pass 1: Collect names
            for path in &toml_files {
                let content = fs::read_to_string(path)?;
                if let Ok(template) = apl_core::package::PackageTemplate::parse(&content) {
                    known_packages.insert(template.package.name.clone());
                    templates.push((path.clone(), template));
                } else if let Ok(pkg) = Package::parse(&content) {
                    known_packages.insert(pkg.package.name.clone());
                }
            }

            // Pass 2: Validate
            for (path, template) in templates {
                let pkg_name = &template.package.name;

                // Check dependencies
                for dep_str in template
                    .dependencies
                    .runtime
                    .iter()
                    .chain(template.dependencies.build.iter())
                    .chain(template.dependencies.optional.iter())
                {
                    let dep = apl_schema::types::PackageName::new(dep_str);
                    if !known_packages.contains(&dep) {
                        eprintln!(
                            "   [ERR] {}: Missing dependency '{}' (path: {})",
                            pkg_name,
                            dep_str,
                            path.display()
                        );
                        errors += 1;
                    }
                }

                // Check Metadata
                if template.package.description.trim().is_empty() {
                    println!("   [WARN] {pkg_name}: Missing description");
                    warnings += 1;
                }
                if template.package.license.trim().is_empty() {
                    println!("   [WARN] {pkg_name}: Missing license");
                    warnings += 1;
                }

                // Check Checksum Policy
                if template.assets.skip_checksums {
                    println!("   [WARN] {pkg_name}: Skips checksum verification");
                    warnings += 1;
                }
            }

            println!("\nSummary:");
            println!("  Packages: {}", known_packages.len());
            println!("  Errors:   {errors}");
            println!("  Warnings: {warnings}");

            if errors > 0 {
                anyhow::bail!("Registry check failed with {errors} errors.");
            } else {
                println!("Registry integrity verified.");
            }
        }
        Commands::Index {
            package,
            full,
            bootstrap,
            verbose,
            output,
        } => {
            let use_path = output.unwrap_or(index_path);
            cli_index(
                &client,
                &registry_dir,
                &use_path,
                package.as_deref(),
                full,
                bootstrap.as_deref(),
                verbose,
            )
            .await?;
        }

        Commands::Keygen => {
            cli_keygen()?;
        }
        Commands::Sign { input, output } => {
            cli_sign(&input, &output)?;
        }
        Commands::Import { from, packages } => {
            apl_core::indexer::import::import_packages(&from, &packages, &registry_dir).await?;
        }
        Commands::Verify { package } => {
            cli_verify(&client, &package).await?;
        }
    }

    Ok(())
}

async fn cli_index(
    client: &reqwest::Client,
    registry_dir: &Path,
    index_path: &Path,
    package_filter: Option<&str>,
    force_full: bool,
    bootstrap_url: Option<&str>,
    verbose: bool,
) -> Result<()> {
    if let Some(url) = bootstrap_url {
        if !index_path.exists() {
            println!("   📥 Bootstrapping index from {url}...");
            match client.get(url).send().await {
                Ok(resp) => {
                    if resp.status().is_success() {
                        let bytes = resp.bytes().await?;
                        fs::write(index_path, bytes)?;
                        println!(
                            "   ✓ Bootstrap successful ({} bytes)",
                            fs::metadata(index_path)?.len()
                        );
                    } else {
                        eprintln!("   ⚠ Bootstrap failed: HTTP {}", resp.status());
                    }
                }
                Err(e) => {
                    eprintln!("   ⚠ Bootstrap failed: {e}");
                }
            }
        } else {
            println!("   ℹ️  Local index exists, skipping bootstrap.");
        }
    }

    println!("Regenerating index...");

    // Use NullReporter for indexing (no live UI output needed for CI)
    let reporter = std::sync::Arc::new(apl_core::NullReporter);

    let index = apl_core::indexer::generate_index_from_registry(
        client,
        registry_dir,
        package_filter,
        force_full,
        verbose,
        reporter.clone(),
    )
    .await?;

    index.save_compressed(index_path)?;

    // Export manifest for install.sh and programmatic consumers
    if let Some(entry) = index.find("apl") {
        if let Some(latest) = entry.latest() {
            let mut urls = serde_json::Map::new();
            for bin in &latest.binaries {
                let key = match bin.arch {
                    apl_schema::Arch::Arm64 => "darwin-arm64",
                    apl_schema::Arch::X86_64 => "darwin-x64",
                    apl_schema::Arch::Universal => "darwin-universal",
                };
                urls.insert(key.to_string(), serde_json::Value::String(bin.url.clone()));
            }

            // Write latest.json with simple key-value pairs for install.sh
            let manifest_path = index_path.with_file_name("latest.json");
            fs::write(&manifest_path, serde_json::to_string_pretty(&urls)?)?;
            println!("   Generated latest.json: {}", manifest_path.display());
        }
    }

    Ok(())
}

fn cli_sign(input: &Path, output: &Path) -> Result<()> {
    use base64::Engine;
    use ed25519_dalek::{Signer, SigningKey};

    let secret_b64 = std::env::var("APL_SIGNING_KEY").context("APL_SIGNING_KEY not set")?;

    let secret_bytes = base64::engine::general_purpose::STANDARD
        .decode(secret_b64.trim())
        .context("Invalid Base64 signing key")?;

    if secret_bytes.len() != 32 {
        anyhow::bail!("APL_SIGNING_KEY must be a 32-byte Ed25519 private key");
    }

    let mut key_arr = [0u8; 32];
    key_arr.copy_from_slice(&secret_bytes);
    let signing_key = SigningKey::from_bytes(&key_arr);

    let data = fs::read(input).context("Failed to read input file")?;
    let signature = signing_key.sign(&data);
    let sig_b64 = base64::engine::general_purpose::STANDARD.encode(signature.to_bytes());

    fs::write(output, sig_b64).context("Failed to write signature file")?;
    println!("✓ Signed {} -> {}", input.display(), output.display());

    Ok(())
}
async fn add_package(client: &reqwest::Client, repo: &str, out_dir: &Path) -> Result<()> {
    use apl_core::package::{
        AssetConfig, AssetSelector, DiscoveryConfig, Hints, InstallSpec, PackageInfoTemplate,
        PackageTemplate,
    };
    use apl_schema::types::PackageName;

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
            select,
            skip_checksums: false,
            checksum_url: None,
        },
        source: None,
        build: None,
        dependencies: apl_core::package::Dependencies::default(),
        install: InstallSpec::default(),
        hints: Hints::default(),
    };

    let target_path = apl_core::indexer::registry_path(out_dir, repo_name);
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

/// Verify a package by parsing and validating its definition.
async fn cli_verify(_client: &reqwest::Client, package_path: &Path) -> Result<()> {
    println!("Verifying {}...", package_path.display());

    // Parse the package definition
    let pkg = Package::from_file(package_path)?;
    let pkg_name = pkg.package.name.clone();
    let bin_list = pkg.install.effective_bin(&pkg_name);

    if bin_list.is_empty() {
        anyhow::bail!("Package defines no binaries to verify");
    }

    // TODO: Implement simplified verification that doesn't depend on apl_cli flow types
    // For now, just parse the package file to validate it
    println!("  ✓ Package definition parsed successfully");
    println!("  ✓ Binaries declared: {}", bin_list.join(", "));
    println!("  ⚠ Full verification (download + smoke test) not yet implemented.");
    println!("    Use `apl install --dry-run {pkg_name}` for full verification.");

    Ok(())
}
