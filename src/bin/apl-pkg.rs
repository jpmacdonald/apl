//! apl-pkg - Unified registry management tool
//! Usage: cargo run --bin apl-pkg -- <command> [args]

use anyhow::Result;
use apl::index::{HashType, IndexBinary, IndexSource, PackageIndex, VersionInfo};
use apl::package::{
    ArtifactFormat, Binary, Dependencies, Hints, InstallSpec, Package, PackageInfo, PackageType,
    Source,
};
use apl::registry::{build_github_client, github};
use apl::{Arch, PackageName, Version};
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
    /// Migrate legacy packages to algorithmic registry templates
    Migrate,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let token = std::env::var("GITHUB_TOKEN").ok();
    let client = build_github_client(token.as_deref())?;

    let packages_dir = std::env::current_dir()?.join("packages");
    let registry_dir = std::env::current_dir()?.join("registry");
    let index_path = std::env::current_dir()?.join("index.bin");

    match cli.command {
        Commands::Add { repos } => {
            for repo in repos {
                println!("Adding {repo}...");
                if let Err(e) = add_package(&client, &repo, &packages_dir).await {
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
                            eprintln!("   Failed to update {file_name}: {e}"); // Keep inline error for context
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
                cli_index(&client, &packages_dir, &index_path).await?;
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
                anyhow::bail!("Registry check failed with {errors} errors.");
            }
        }
        Commands::Index => {
            cli_index(&client, &packages_dir, &index_path).await?;
        }
        Commands::Migrate => {
            cli_migrate(&packages_dir, &registry_dir).await?;
        }
    }

    Ok(())
}

/// Compute the sharded registry path for a package name
/// - Single-letter names: registry/1/{name}.toml
/// - Multi-letter names: registry/{first-two-letters}/{name}.toml
fn registry_path(registry_dir: &Path, name: &str) -> std::path::PathBuf {
    let prefix = if name.len() == 1 {
        "1".to_string()
    } else {
        name[..2].to_lowercase()
    };
    registry_dir.join(prefix).join(format!("{name}.toml"))
}

/// Simple persistent hash cache to avoid re-downloading thousands of versions
#[derive(serde::Serialize, serde::Deserialize)]
struct CachedEntry {
    hash: String,
    hash_type: HashType,
    timestamp: u64,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct HashCache {
    /// Map of URL -> CachedEntry (hash + type + timestamp)
    entries: HashMap<String, CachedEntry>,
}

impl HashCache {
    fn load() -> Self {
        let path = apl::apl_home().join("cache").join("hashes.json");
        if path.exists() {
            if let Ok(content) = fs::read_to_string(&path) {
                if let Ok(cache) = serde_json::from_str(&content) {
                    return cache;
                }
            }
        }
        Self::default()
    }

    fn save(&self) -> Result<()> {
        let cache_dir = apl::apl_home().join("cache");
        fs::create_dir_all(&cache_dir)?;
        let path = cache_dir.join("hashes.json");
        let content = serde_json::to_string_pretty(&self)?;
        fs::write(path, content)?;
        Ok(())
    }

    fn get(&self, url: &str) -> Option<(String, HashType)> {
        self.entries.get(url).map(|e| (e.hash.clone(), e.hash_type))
    }

    fn insert(&mut self, url: String, hash: String, hash_type: HashType) {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        self.entries.insert(
            url,
            CachedEntry {
                hash,
                hash_type,
                timestamp,
            },
        );
    }
}

/// Parse vendor checksum files (sha256sum.txt, SHA256SUMS, etc.)
/// This is the CRITICAL optimization that prevents downloading 600GB of binaries
async fn fetch_and_parse_checksum(
    client: &reqwest::Client,
    checksum_url: &str,
    asset_url: &str,
) -> Result<String> {
    let resp = client.get(checksum_url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("Checksum file not found: {checksum_url}");
    }

    let text = resp.text().await?;

    // Extract filename from asset URL
    let filename = asset_url
        .split('/')
        .next_back()
        .ok_or_else(|| anyhow::anyhow!("Invalid asset URL"))?;

    // Parse checksum files (formats: "hash  filename" or "hash filename" or "hash *filename")
    for line in text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 {
            let hash = parts[0];
            let file = parts[1].trim_start_matches('*'); // Remove leading * if present

            // Match exact filename or if the line ends with our filename
            if file == filename || file.ends_with(filename) {
                return Ok(hash.to_string());
            }
        }
    }

    anyhow::bail!("Hash not found in checksum file for {filename}")
}

/// Discover versions from a template's discovery configuration
async fn discover_versions(
    client: &reqwest::Client,
    discovery: &apl::package::DiscoveryConfig,
) -> Result<Vec<String>> {
    use apl::package::DiscoveryConfig;

    match discovery {
        DiscoveryConfig::GitHub {
            github,
            tag_pattern,
            semver_only,
            include_prereleases,
        } => {
            let (owner, repo) = github
                .split_once('/')
                .ok_or_else(|| anyhow::anyhow!("Invalid GitHub repo format: {github}"))?;

            let releases = github::fetch_all_releases(client, owner, repo).await?;

            let mut versions = Vec::new();
            for release in releases {
                // Filter prereleases
                if !include_prereleases && release.prerelease {
                    continue;
                }

                // Extract version from tag using pattern
                let version = extract_version_from_tag(&release.tag_name, tag_pattern);

                // Validate semver if required
                if *semver_only && semver::Version::parse(&version).is_err() {
                    continue;
                }

                versions.push(version);
            }

            Ok(versions)
        }
        DiscoveryConfig::Manual { manual } => Ok(manual.clone()),
    }
}

fn extract_version_from_tag(tag: &str, pattern: &str) -> String {
    if pattern == "{{version}}" {
        // If the tag starts with 'v', but pattern is just {{version}},
        // we should probably still try to parse it as a version.
        // Let's use the shared strip_tag_prefix logic but we don't have the repo name here easily.
        // For now, simple 'v' stripping is usually what's needed.
        tag.strip_prefix('v').unwrap_or(tag).to_string()
    } else {
        tag.replace(&pattern.replace("{{version}}", ""), "")
    }
}

async fn cli_index(client: &reqwest::Client, packages_dir: &Path, index_path: &Path) -> Result<()> {
    println!("Regenerating index...");

    // Check if we should use new registry/ or old packages/
    let registry_dir = Path::new("registry");
    let index = if registry_dir.exists() && registry_dir.is_dir() {
        println!("   Using algorithmic registry (registry/)...");
        generate_index_from_registry(client, registry_dir).await?
    } else {
        println!("   Using legacy packages directory...");
        generate_index_from_dir(client, packages_dir).await?
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
            let repo = guess_github_repo(&pkg.source.url);

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
                        url_template: guess_url_template(
                            &pkg.source.url,
                            &pkg.package.version,
                            &repo,
                        ),
                        targets: guess_targets(&pkg),
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

                let target_path = registry_path(registry_dir, &name);
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

fn guess_github_repo(url: &str) -> Option<String> {
    if url.contains("github.com") {
        let parts: Vec<&str> = url.split('/').collect();
        if parts.len() >= 5 {
            return Some(format!("{}/{}", parts[3], parts[4]));
        }
    }
    None
}

fn guess_url_template(url: &str, version: &str, _repo: &str) -> String {
    let mut template = url.replace(version, "{{version}}");

    // Replace architecture strings with {{target}}
    let arch_patterns = [
        "aarch64-apple-darwin",
        "x86_64-apple-darwin",
        "arm64-apple-darwin",
        "aarch64",
        "x86_64",
        "arm64",
        "amd64",
    ];

    for pattern in arch_patterns {
        if template.contains(pattern) {
            template = template.replace(pattern, "{{target}}");
            break; // Only replace once
        }
    }

    template
}

fn guess_targets(pkg: &apl::package::Package) -> Option<HashMap<String, String>> {
    let mut targets = HashMap::new();

    for (arch, bin) in &pkg.binary {
        let arch_name = match arch {
            apl::Arch::Arm64 => "arm64",
            apl::Arch::X86_64 => "x86_64",
        };

        // Try to find arch-specific string in URL and use full triple if available
        if bin.url.contains("aarch64-apple-darwin") {
            targets.insert(arch_name.to_string(), "aarch64-apple-darwin".to_string());
        } else if bin.url.contains("x86_64-apple-darwin") {
            targets.insert(arch_name.to_string(), "x86_64-apple-darwin".to_string());
        } else if bin.url.contains("arm64-apple-darwin") {
            targets.insert(arch_name.to_string(), "arm64-apple-darwin".to_string());
        } else if bin.url.contains("aarch64") {
            targets.insert(arch_name.to_string(), "aarch64".to_string());
        } else if bin.url.contains("arm64") {
            targets.insert(arch_name.to_string(), "arm64".to_string());
        } else if bin.url.contains("x86_64") {
            targets.insert(arch_name.to_string(), "x86_64".to_string());
        } else if bin.url.contains("amd64") {
            targets.insert(arch_name.to_string(), "amd64".to_string());
        }
    }

    if targets.is_empty() {
        None
    } else {
        Some(targets)
    }
}

/// Generate index from algorithmic registry templates
async fn generate_index_from_registry(
    client: &reqwest::Client,
    registry_dir: &Path,
) -> Result<PackageIndex> {
    use apl::package::PackageTemplate;

    let mut hash_cache = HashCache::load();
    let mut index = PackageIndex::default();
    let mut error_count = 0;

    // Scan all prefix directories
    for prefix_entry in fs::read_dir(registry_dir)? {
        let prefix_entry = prefix_entry?;
        let prefix_path = prefix_entry.path();

        if !prefix_path.is_dir() {
            continue;
        }

        // Scan templates in this prefix directory
        for template_entry in fs::read_dir(&prefix_path)? {
            let template_entry = template_entry?;
            let template_path = template_entry.path();

            if template_path.extension().is_some_and(|e| e == "toml") {
                println!("   Processing {}...", template_path.display());

                // Parse template
                let toml_str = fs::read_to_string(&template_path)?;
                let template: PackageTemplate = toml::from_str(&toml_str)?;

                // Discover versions
                let versions = match discover_versions(&client, &template.discovery).await {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("     ⚠ Failed to discover versions: {e}");
                        error_count += 1;
                        continue;
                    }
                };

                println!("     Found {} versions", versions.len());

                // Hydrate each version
                let mut releases = Vec::new();
                for version in versions.iter() {
                    let mut binaries = Vec::new();

                    // Get targets from template
                    if let Some(targets) = &template.assets.targets {
                        for (arch_name, target_str) in targets {
                            // Apply templates
                            let url = template
                                .assets
                                .url_template
                                .replace("{{version}}", version)
                                .replace("{{target}}", target_str);

                            // Try cache first
                            let (hash, hash_type) = if let Some(cached) = hash_cache.get(&url) {
                                cached
                            } else if let Some(checksum_template) = &template.checksums.url_template
                            {
                                let checksum_url = checksum_template
                                    .replace("{{version}}", version)
                                    .replace("{{target}}", target_str);

                                match fetch_and_parse_checksum(&client, &checksum_url, &url).await {
                                    Ok(h) => {
                                        let hash_type = template
                                            .checksums
                                            .vendor_type
                                            .unwrap_or(HashType::Sha256);
                                        hash_cache.insert(url.clone(), h.clone(), hash_type);
                                        (h, hash_type)
                                    }
                                    Err(e) => {
                                        // Skip version if checksum fetch fails (no fallback to binary download)
                                        eprintln!(
                                            "     ⚠ Checksum fetch failed for {}: {}",
                                            url, e
                                        );
                                        continue;
                                    }
                                }
                            } else if template.checksums.skip {
                                // Explicitly skipped - use empty hash (will be verified at install time)
                                ("".to_string(), HashType::Blake3)
                            } else {
                                // No checksum template and skip=false - skip version with warning
                                eprintln!("     ⚠ No checksum config for {} - skipping", url);
                                continue;
                            };

                            binaries.push(IndexBinary {
                                arch: arch_name.clone(),
                                url,
                                hash,
                                hash_type,
                            });
                        }
                    }

                    if !binaries.is_empty() {
                        releases.push(VersionInfo {
                            version: version.clone(),
                            binaries,
                            deps: vec![],
                            build_deps: vec![],
                            build_script: String::new(),
                            bin: template.install.bin.clone(),
                            hints: template.hints.post_install.clone(),
                            app: template.install.app.clone(),
                            source: None,
                        });
                    }
                }

                if !releases.is_empty() {
                    let type_str = match template.package.type_ {
                        apl::package::PackageType::Cli => "cli",
                        apl::package::PackageType::App => "app",
                    };

                    index.upsert(apl::index::IndexEntry {
                        name: template.package.name.to_string(),
                        description: template.package.description.clone(),
                        homepage: template.package.homepage.clone(),
                        type_: type_str.to_string(),
                        releases,
                    });
                }
            }
        }
    }

    if error_count > 0 {
        hash_cache.save()?; // Save whatever progress we made
        anyhow::bail!("Index generation failed with {} errors", error_count);
    }

    hash_cache.save()?;
    Ok(index)
}

async fn generate_index_from_dir(client: &reqwest::Client, dir: &Path) -> Result<PackageIndex> {
    let mut index = PackageIndex::new();
    index.updated_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;

    let mut hash_cache = HashCache::load();

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().is_some_and(|ext| ext == "toml") {
            let pkg = Package::from_file(&path)
                .map_err(|e| anyhow::anyhow!("Failed to parse {}: {}", path.display(), e))?;

            // Decide if we should fetch history for this package
            let mut all_versions = Vec::new();

            // 1. Always include the version specifically defined in TOML (manual override / baseline)
            all_versions.push(pkg.clone());

            // 2. NEW: Process Manual Manual Version List (URL Templating)
            if let (Some(versions), Some(template)) =
                (&pkg.source.versions, &pkg.source.url_template)
            {
                println!(
                    "   Processing {} manual versions for {}",
                    versions.len(),
                    pkg.package.name
                );

                for ver_str in versions {
                    // Skip if it matches the main version (already added)
                    if pkg.package.version == *ver_str {
                        continue;
                    }

                    // Apply template
                    let url = template.replace("{{version}}", ver_str);

                    // Check cache or download
                    let (hash, _hash_type) = if let Some(cached) = hash_cache.get(&url) {
                        cached
                    } else {
                        println!("       Downloading {ver_str}...");
                        match client.get(&url).send().await {
                            Ok(resp) => {
                                if let Ok(bytes) = resp.bytes().await {
                                    let h = blake3::hash(&bytes).to_hex().to_string();
                                    hash_cache.insert(url.clone(), h.clone(), HashType::Blake3);
                                    (h, HashType::Blake3)
                                } else {
                                    eprintln!("       Failed to read bytes for {url}");
                                    continue;
                                }
                            }
                            Err(e) => {
                                eprintln!("       Failed to download {url}: {e}");
                                continue;
                            }
                        }
                    };

                    // Create version package
                    let mut ver_pkg = pkg.clone();
                    ver_pkg.package.version = Version::from(ver_str.clone());

                    // For manual versions, we assume the same URL for all architectures if not specified otherwise,
                    // OR we assume the template handles arch if we want to get fancy later.
                    // For now, let's assume the template produces a universal artifact OR matches the source format.
                    // We need to populate `binary` map.

                    // If the template produces a generic URL, we might need to assume it's valid for the supported architectures
                    // defined in the base package?
                    // Actually, usually `url_template` is used in `[source]`, implying it's the source distribution (or universal package).
                    // So we update `ver_pkg.source` and let the indexer handle it as a source-only release if no binaries?
                    // BUT `apl` installation prefers binaries.

                    // Let's create a "Binary" entry for the current arch IF the base package has one,
                    // or just put it in `source`?
                    // APL index expects `binaries` list.

                    // Constraint: The current simple template `url_template` probably points to the same kind of artifact as `[source]`.
                    // So we update `source`.

                    ver_pkg.source = Source {
                        url: url.clone(),
                        blake3: hash.clone(),
                        format: pkg.source.format.clone(),
                        strip_components: pkg.source.strip_components,
                        url_template: pkg.source.url_template.clone(),
                        versions: None,
                    };

                    // For the binary map:
                    // If the original package had binaries, we might want to try to construct binaries for them too?
                    // This is tricky without per-arch templates.
                    // SIMPLE START: Assume the manual version is a "Source" distribution (like the main `[source]` block).
                    // The client can install from source.
                    // However, if the user provides a direct binary URL in the template (e.g. .pkg), it should be treated as a binary.

                    // Better approach:
                    // If `pkg.binary` is empty, this is a source/universal package.
                    // If `pkg.binary` has entries, we can't easily guess the URL for those binaries using a single template.
                    // UNLESS the template is meant for the *source* only.

                    // Re-reading the task: "Enable `apl install package@version`".
                    // If I only populate `source`, `apl` will try to build it?
                    // `apl` installs from `binaries` list or `source`.

                    // Let's look at `aws.toml`. It has `[source]` pointing to a `.pkg`.
                    // `[Binary]` is usually for arch-specific pre-compiled.
                    // `aws` has ONLY `[source]`.
                    // So updating `source` and letting `binary` map be empty (or derived) is correct for `aws`.

                    // If we have architectures, we might want to support `url_template_arm64` etc later.
                    // For now, let's just populate `ver_pkg.source`.
                    // AND if the base package treats `source` as the binary (e.g. valid format), we should probably
                    // duplicate it into `binary` map for the "index" to see it as an available binary?

                    // Wait, `generate_index_from_dir` lines 430+ converts `p.binary` map to `IndexBinary`.
                    // It also populates `source: Some(...)`.
                    // If `p.binary` is empty, `binaries` list in index is empty.
                    // `install.rs` logic: `resolve_package` -> checks `binaries`. If none, checks `source`.
                    // So populating `source` is sufficient for installation!

                    // However, if the user wants to treat this as a "binary" install (ignoring build),
                    // the `source` must have a format that `apl` can install directly (like Pkg, App, or simple binary).
                    // This is true for AWS (.pkg).

                    // Optimization: If the `base` package has NO binary map (only source), we are good.
                    // If the `base` package HAS binary map, and we only provide source, `apl` will try to build from source.
                    // This is acceptable behavior for "manual version override" MVP.

                    // Clear binary map to avoid misleading "missing" binaries if we can't derive them.
                    ver_pkg.binary = HashMap::new();

                    all_versions.push(ver_pkg);
                }
            }

            // 3. If it's a GitHub source, fetch history (existing logic)
            if let Some(homepage) = pkg.package.homepage.strip_prefix("https://github.com/") {
                let parts: Vec<&str> = homepage.split('/').collect();
                if parts.len() >= 2 {
                    let owner = parts[0];
                    let repo = parts[1];
                    println!(
                        "   Fetching history for {} ({}/{})",
                        pkg.package.name, owner, repo
                    );

                    match github::fetch_all_releases(&client, owner, repo).await {
                        Ok(releases) => {
                            println!("     Found {} releases", releases.len());

                            for release in releases.iter().take(20) {
                                // Safety limit
                                let ver_str = github::strip_tag_prefix(&release.tag_name, repo);

                                // Skip if it matches the TOML version (already added)
                                if pkg.package.version == ver_str {
                                    continue;
                                }

                                // Skip invalid semver
                                if semver::Version::parse(&ver_str).is_err() {
                                    continue;
                                }

                                // Clone the package struct and modify for this version
                                let mut ver_pkg = pkg.clone();
                                ver_pkg.package.version = Version::from(ver_str.clone());

                                // Find assets
                                let (arm64, x86) = github::find_macos_assets(release, repo);

                                if arm64.is_none() && x86.is_none() {
                                    continue;
                                }

                                // Prepare binary map
                                let mut binary_map = HashMap::new();
                                let mut found_any = false;

                                // ARM64
                                if let Some(asset) = arm64 {
                                    if let Some((hash, _ht)) =
                                        hash_cache.get(&asset.browser_download_url)
                                    {
                                        binary_map.insert(
                                            Arch::Arm64,
                                            Binary {
                                                url: asset.browser_download_url.clone(),
                                                blake3: hash,
                                                format: ArtifactFormat::Binary, // Generic, unused by index
                                                arch: Arch::Arm64,
                                                macos: "14.0".to_string(),
                                            },
                                        );
                                        found_any = true;
                                    } else {
                                        // Cache miss - we need to download (skip for now to avoid slowdown)
                                        // In a real implementation we would download here.
                                        // For prototype: we skip un-cached historical versions to be safe?
                                        // No, let's download!
                                        println!("       Downloading {ver_str}...");
                                        match client.get(&asset.browser_download_url).send().await {
                                            Ok(resp) => {
                                                if let Ok(bytes) = resp.bytes().await {
                                                    let hash =
                                                        blake3::hash(&bytes).to_hex().to_string();
                                                    hash_cache.insert(
                                                        asset.browser_download_url.clone(),
                                                        hash.clone(),
                                                        HashType::Blake3,
                                                    );

                                                    binary_map.insert(
                                                        Arch::Arm64,
                                                        Binary {
                                                            url: asset.browser_download_url.clone(),
                                                            blake3: hash,
                                                            format: ArtifactFormat::Binary,
                                                            arch: Arch::Arm64,
                                                            macos: "14.0".to_string(),
                                                        },
                                                    );
                                                    found_any = true;
                                                }
                                            }
                                            Err(_) => continue,
                                        }
                                    }
                                }

                                // x86_64
                                if let Some(asset) = x86 {
                                    // Same logic for x86... (Simplified: blindly assuming if arm64 succeeded or independent)
                                    // For brevity in this edit, let's just use the arm64 one if consistent, or replicate steps.
                                    // Replicating steps for correctness:
                                    if let Some((hash, _ht)) =
                                        hash_cache.get(&asset.browser_download_url)
                                    {
                                        binary_map.insert(
                                            Arch::X86_64,
                                            Binary {
                                                url: asset.browser_download_url.clone(),
                                                blake3: hash,
                                                format: ArtifactFormat::Binary,
                                                arch: Arch::X86_64,
                                                macos: "14.0".to_string(),
                                            },
                                        );
                                        found_any = true;
                                    } else {
                                        match client.get(&asset.browser_download_url).send().await {
                                            Ok(resp) => {
                                                if let Ok(bytes) = resp.bytes().await {
                                                    let hash =
                                                        blake3::hash(&bytes).to_hex().to_string();
                                                    hash_cache.insert(
                                                        asset.browser_download_url.clone(),
                                                        hash.clone(),
                                                        HashType::Blake3,
                                                    );

                                                    binary_map.insert(
                                                        Arch::X86_64,
                                                        Binary {
                                                            url: asset.browser_download_url.clone(),
                                                            blake3: hash,
                                                            format: ArtifactFormat::Binary,
                                                            arch: Arch::X86_64,
                                                            macos: "14.0".to_string(),
                                                        },
                                                    );
                                                    found_any = true;
                                                }
                                            }
                                            Err(_) => continue,
                                        }
                                    }
                                }

                                if found_any {
                                    ver_pkg.binary = binary_map;
                                    all_versions.push(ver_pkg);
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("     Failed to fetch history: {e}");
                        }
                    }
                }
            }

            // Process all versions into the index
            let mut releases = Vec::new();

            for p in all_versions {
                let binaries: Vec<IndexBinary> = p
                    .binary
                    .iter()
                    .map(|(arch, binary)| IndexBinary {
                        arch: arch.as_str().to_string(),
                        url: binary.url.clone(),
                        hash: binary.blake3.clone(),
                        hash_type: HashType::Blake3,
                    })
                    .collect();

                let release = VersionInfo {
                    version: p.package.version.to_string(),
                    binaries,
                    deps: p.dependencies.runtime.clone(),
                    build_deps: p.dependencies.build.clone(),
                    build_script: p
                        .build
                        .as_ref()
                        .map(|b| b.script.clone())
                        .unwrap_or_default(),
                    bin: p.install.bin.clone(),
                    hints: p.hints.post_install.clone(),
                    app: p.install.app.clone(),
                    source: Some(IndexSource {
                        url: p.source.url.clone(),
                        hash: p.source.blake3.clone(),
                        hash_type: HashType::Blake3,
                    }),
                };
                releases.push(release);
            }

            // Upsert into index (we now have a list of releases)
            // existing PackageIndex.upsert_release expects one release at a time.
            // We can call it in a loop.
            let type_str = match pkg.package.type_ {
                PackageType::Cli => "cli",
                PackageType::App => "app",
            };

            for rel in releases {
                index.upsert_release(&pkg.package.name, &pkg.package.description, type_str, rel);
            }
        }
    }

    // Save cache at the end
    hash_cache.save()?;
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
        println!("   ARM64 BLAKE3: {hash}");

        binary_map.insert(
            Arch::Arm64,
            Binary {
                url: url.clone(),
                blake3: hash.clone(),
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
        println!("   x86_64 BLAKE3: {hash}");

        binary_map.insert(
            Arch::X86_64,
            Binary {
                url: url.clone(),
                blake3: hash.clone(),
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
                    blake3: source_hash,
                    format: format.clone(),
                    strip_components,
                    url_template: None,
                    versions: None,
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
            blake3: source_hash,
            format: format.clone(),
            strip_components,
            url_template: None,
            versions: None,
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
    let toml_path = out_dir.join(format!("{repo_name}.toml"));
    fs::write(&toml_path, toml_content)?;
    println!("   Created {}", toml_path.display());

    Ok(())
}
