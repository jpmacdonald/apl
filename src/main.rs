use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use reqwest::Client;
use tracing_subscriber::EnvFilter;

use dl::cas::Cas;
use dl::db::StateDb;
use dl::downloader::download_and_verify;
use dl::formula::Formula;
use dl::{bin_path, dl_home};

#[derive(Parser)]
#[command(name = "dl")]
#[command(author, version, about = "dl - A modern package manager for macOS")]
struct Cli {
    /// Show what would happen without making changes
    #[arg(long, global = true)]
    dry_run: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Install a package
    Install {
        /// Package name(s) or path to formula file
        #[arg(required = true)]
        packages: Vec<String>,
        /// Only install packages pinned in dl.lock
        #[arg(long)]
        locked: bool,
    },
    /// Remove a package
    Remove {
        /// Package name(s) to remove
        #[arg(required = true)]
        packages: Vec<String>,
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
        /// File(s) to hash
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    /// Generate or update dl.lock from installed packages
    Lock,
    /// Search available packages
    Search {
        /// Search query
        query: String,
    },
    /// Generate index.msgpack from formulas directory
    IndexGen {
        /// Directory containing formula files
        #[arg(default_value = "formulas")]
        formulas_dir: PathBuf,
        /// Output file
        #[arg(default_value = "index.msgpack")]
        output: PathBuf,
    },
    /// Garbage collect orphaned CAS blobs
    Gc,
    /// Update package index from CDN
    Update {
        /// CDN URL for index
        #[arg(long, env = "DL_INDEX_URL", default_value = "https://raw.githubusercontent.com/jpmacdonald/distill/main/index.msgpack")]
        url: String,
    },
    /// Upgrade installed packages to latest versions
    Upgrade {
        /// Specific packages to upgrade (all if omitted)
        packages: Vec<String>,
    },
    /// Formula management commands
    Formula {
        #[command(subcommand)]
        command: FormulaCommands,
    },
    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// Update dl itself to the latest version
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
pub enum FormulaCommands {
    /// Create a new formula template
    New {
        /// Package name
        name: String,
        /// Optional directory to save the formula in
        #[arg(short, long, default_value = "formulas")]
        output_dir: PathBuf,
    },
    /// Validate a formula file
    Check {
        /// Path to formula file
        path: PathBuf,
    },
    /// Bump a formula version and update hashes
    Bump {
        /// Formula file to bump
        path: PathBuf,
        /// New version
        #[arg(long)]
        version: String,
        /// New bottle URL for current arch
        #[arg(long)]
        url: String,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing (only if RUST_LOG is set - faster cold start)
    if std::env::var("RUST_LOG").is_ok() {
        tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::from_default_env())
            .init();
    }

    // Ensure dl directories exist
    std::fs::create_dir_all(dl_home())?;
    std::fs::create_dir_all(bin_path())?;

    let cli = Cli::parse();
    let dry_run = cli.dry_run;

    match cli.command {
        Commands::Install { packages, locked } => {
            install_packages(&packages, dry_run, locked).await?;
        }
        Commands::Remove { packages } => {
            remove_packages(&packages, dry_run)?;
        }
        Commands::List => {
            list_packages()?;
        }
        Commands::Info { package } => {
            show_package_info(&package)?;
        }
        Commands::Hash { files } => {
            hash_files(&files)?;
        }
        Commands::Lock => {
            generate_lockfile(dry_run)?;
        }
        Commands::Search { query } => {
            search_packages(&query)?;
        }
        Commands::IndexGen { formulas_dir, output } => {
            generate_index(&formulas_dir, &output)?;
        }
        Commands::Gc => {
            garbage_collect(dry_run)?;
        }
        Commands::Update { url } => {
            update_index(&url, dry_run).await?;
        }
        Commands::Upgrade { packages } => {
            upgrade_packages(&packages, dry_run).await?;
        }
        Commands::Formula { command } => {
            match command {
                FormulaCommands::New { name, output_dir } => {
                    formula_new(&name, &output_dir)?;
                }
                FormulaCommands::Check { path } => {
                    formula_check(&path)?;
                }
                FormulaCommands::Bump { path, version, url } => {
                    formula_bump(&path, &version, &url).await?;
                }
            }
        }
        Commands::Completions { shell } => {
            generate_completions(shell);
        }
        Commands::SelfUpdate => {
            self_update_dl(dry_run).await?;
        }
        Commands::Run { package, args } => {
            run_package(&package, &args, dry_run).await?;
        }
    }

    Ok(())
}

/// Prepared package ready for finalization
pub struct PreparedPackage {
    pub name: String,
    pub version: String,
    pub download_path: PathBuf,
    pub bin_list: Vec<String>,
    pub formula: Option<Formula>,
    pub _temp_dir: Option<tempfile::TempDir>,
}

/// Install one or more packages (parallel downloads, sequential install)
async fn install_packages(packages: &[String], dry_run: bool, locked: bool) -> Result<()> {
    use futures::future::join_all;
    use dl::lockfile::Lockfile;
    use dl::resolver::resolve_dependencies;
    use dl::index::PackageIndex;

    let client = Client::new();
    let cas = Cas::new().context("Failed to initialize CAS")?;
    let db = StateDb::open().context("Failed to open state database")?;

    // Load index for resolution
    let index_path = dl_home().join("index.msgpack");
    let index = if index_path.exists() {
        PackageIndex::load(&index_path).ok()
    } else {
        None
    };

    // Load lockfile if --locked or exists
    let lockfile = if locked {
        if !Lockfile::exists_default() {
            bail!("No dl.lock found. Run 'dl lock' first or remove --locked flag.");
        }
        Some(Lockfile::load_default().context("Failed to load dl.lock")?)
    } else {
        Lockfile::load_default().ok()
    };

    // Phase 1: Dependency Resolution
    let resolved_packages = if let Some(idx) = &index {
        // We need to handle local formula files in the input
        // For now, assume names are in the index. 
        // TODO: Handle local formula dependencies correctly in resolver.
        resolve_dependencies(packages, idx).context("Failed to resolve dependencies")?
    } else {
        // No index, just install what was requested (best effort)
        packages.to_vec()
    };

    if resolved_packages.len() > packages.len() {
        println!("📦 Resolved {} packages (including dependencies)", resolved_packages.len());
    }

    // Phase 2: Parallel downloads
    println!("📦 Downloading {} packages...", resolved_packages.len());
    
    let download_futures: Vec<_> = resolved_packages
        .iter()
        .map(|pkg| {
            let client = &client;
            let lockfile = &lockfile;
            async move {
                prepare_download(client, pkg, dry_run, lockfile.as_ref()).await
            }
        })
        .collect();

    let results = join_all(download_futures).await;

    // Collect successful downloads
    let mut prepared_map = std::collections::HashMap::new();
    for (pkg, result) in resolved_packages.iter().zip(results) {
        match result {
            Ok(Some(p)) => {
                prepared_map.insert(pkg.clone(), p);
            }
            Ok(None) => {} // Already installed or dry-run
            Err(e) => eprintln!("✗ Failed to download {}: {}", pkg, e),
        }
    }

    // Phase 3: Sequential installation in resolved order
    for pkg_name in resolved_packages {
        if let Some(prepared) = prepared_map.remove(&pkg_name) {
            finalize_package(&cas, &db, &prepared, dry_run)
                .context(format!("Failed to install {}", pkg_name))?;
        }
    }

    // Update lockfile with newly installed packages
    if !dry_run {
        if let Some(lf) = lockfile {
            lf.save_default().ok();
        }
    }

    Ok(())
}

/// Prepare a package download (parallel-safe, no DB access)
async fn prepare_download(
    client: &Client,
    pkg: &str,
    dry_run: bool,
    lockfile: Option<&dl::lockfile::Lockfile>,
) -> Result<Option<PreparedPackage>> {
    use dl::index::PackageIndex;
    
    let formula_path = PathBuf::from(pkg);
    
    // Try loading as file first, then fall back to index lookup
    let (name, version, bottle_url, bottle_hash, bin_list, formula_opt) = if formula_path.exists() {
        // Load from formula file
        let formula = Formula::from_file(&formula_path)?;
        let bottle = formula.bottle_for_current_arch()
            .context("No bottle for current arch")?;
        (
            formula.package.name.clone(),
            formula.package.version.clone(),
            bottle.url.clone(),
            bottle.blake3.clone(),
            formula.install.bin.clone(),
            Some(formula),
        )
    } else {
        // Try to find in index
        let index_path = dl_home().join("index.msgpack");
        if !index_path.exists() {
            bail!("Package '{}' not found. Run 'dl update' to fetch package index.", pkg);
        }
        
        let index = PackageIndex::load(&index_path)
            .context("Failed to load index")?;
        
        let entry = index.find(pkg)
            .context(format!("Package '{}' not found in index", pkg))?;
        
        // Find bottle for current arch
        let current_arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x86_64" };
        let bottle = entry.bottles.iter()
            .find(|b| b.arch.contains(current_arch) || b.arch == current_arch)
            .context(format!("No bottle for {} on {}", pkg, current_arch))?;
        
        (
            entry.name.clone(),
            entry.version.clone(),
            bottle.url.clone(),
            bottle.blake3.clone(),
            entry.bin.clone(),
            None, // No formula file
        )
    };

    // Check lockfile for pinned version
    if let Some(lf) = lockfile {
        if let Some(locked) = lf.find(&name) {
            if locked.version != version {
                bail!("Locked to {} but index has {}", locked.version, version);
            }
        }
    }

    if dry_run {
        println!("Would install: {} {}", name, version);
        println!("  Source: {}", bottle_url);
        return Ok(None);
    }

    // Download to temp
    let temp_dir = tempfile::tempdir()?;
    let url_filename = bottle_url.split('/').last().unwrap_or("download");
    let temp_file = temp_dir.path().join(url_filename);

    download_and_verify(client, &bottle_url, &temp_file, &bottle_hash)
        .await
        .context("Download failed")?;

    // Create a minimal formula if we don't have one (for install.bin detection)
    let formula = formula_opt.unwrap_or_else(|| {
        Formula {
            package: dl::formula::PackageInfo {
                name: name.clone(),
                version: version.clone(),
                description: String::new(),
                homepage: String::new(),
                license: String::new(),
            },
            source: dl::formula::Source {
                url: String::new(),
                blake3: String::new(),
                strip_components: 0,
            },
            bottle: std::collections::HashMap::new(),
            dependencies: dl::formula::Dependencies::default(),
            install: dl::formula::InstallSpec {
                bin: if bin_list.is_empty() { vec![name.clone()] } else { bin_list.clone() },
                lib: vec![],
                include: vec![],
                script: String::new(),
            },
        }
    });

    Ok(Some(PreparedPackage {
        name: name.clone(),
        version: version.clone(),
        download_path: temp_file,
        bin_list,
        formula: Some(formula),
        _temp_dir: Some(temp_dir),
    }))
}

/// Finalize package installation (sequential, uses DB)
fn finalize_package(
    cas: &Cas,
    db: &StateDb,
    pkg: &PreparedPackage,
    _dry_run: bool,
) -> Result<()> {
    println!("📦 Installing {} {}...", pkg.name, pkg.version);

    // Check if already installed
    if db.get_package(&pkg.name)?.is_some() {
        println!("  {} already installed, skipping", pkg.name);
        return Ok(());
    }

    // Extract
    let extract_dir = pkg.download_path.parent().unwrap().join("extracted");
    let extracted = dl::extractor::extract_auto(&pkg.download_path, &extract_dir)?;

    println!("  📂 Extracted {} files", extracted.len());

    // Store in CAS + link binaries
    let mut package_hash = String::new();
    let mut files_to_record = Vec::new();

    let is_raw = extracted.len() == 1 && 
        dl::extractor::detect_format(&pkg.download_path) == dl::extractor::ArchiveFormat::RawBinary;

    for file in &extracted {
        let hash = cas.store_file(&file.absolute_path)?;
        if package_hash.is_empty() { package_hash = hash.clone(); }

        let file_name = file.relative_path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        
        let formula = pkg.formula.as_ref().expect("Formula must be set in PreparedPackage");
        let bins: Vec<&str> = if is_raw && !formula.install.bin.is_empty() {
            formula.install.bin.iter().map(|s| s.as_str()).collect()
        } else if formula.install.bin.contains(&file_name.to_string()) {
            vec![file_name]
        } else if formula.install.bin.is_empty() && file.is_executable {
            vec![file_name]
        } else {
            continue;
        };

        for bin in bins {
            let target = bin_path().join(bin);
            if target.exists() { std::fs::remove_file(&target).ok(); }
            cas.link_to(&hash, &target)?;
            #[cfg(unix)] {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755))?;
            }
            println!("  → {}", bin);
            files_to_record.push((target.to_string_lossy().to_string(), hash.clone()));
        }
    }

    // Record in DB
    db.install_package(&pkg.name, &pkg.version, &package_hash)?;
    for (path, hash) in &files_to_record {
        db.add_file(path, &pkg.name, hash)?;
    }

    println!("✓ {} installed", pkg.name);
    Ok(())
}

/// Generate dl.lock from installed packages
fn generate_lockfile(dry_run: bool) -> Result<()> {
    use dl::lockfile::{Lockfile, LockedPackage};

    let db = StateDb::open().context("Failed to open database")?;
    let packages = db.list_packages()?;

    if packages.is_empty() {
        println!("No packages installed. Nothing to lock.");
        return Ok(());
    }

    let mut lockfile = Lockfile::new();
    
    for pkg in &packages {
        // Note: We don't have URL stored in DB, would need formula to get it
        // For now, just record what we have
        lockfile.add_package(LockedPackage {
            name: pkg.name.clone(),
            version: pkg.version.clone(),
            blake3: pkg.blake3.clone(),
            url: String::new(), // Would need to re-resolve from formula
            arch: std::env::consts::ARCH.to_string(),
        });
    }

    if dry_run {
        println!("Would generate dl.lock with {} packages:", packages.len());
        for pkg in &packages {
            println!("  {} {}", pkg.name, pkg.version);
        }
    } else {
        lockfile.save_default()?;
        println!("✓ Generated dl.lock with {} packages", packages.len());
    }

    Ok(())
}

/// Remove one or more packages
fn remove_packages(packages: &[String], dry_run: bool) -> Result<()> {
    let db = StateDb::open().context("Failed to open state database")?;

    for pkg in packages {
        // Get files first for dry-run
        let files = db.get_package_files(pkg).unwrap_or_default();
        
        if dry_run {
            if db.get_package(pkg)?.is_some() {
                println!("Would remove: {}", pkg);
                for f in &files {
                    println!("  ✗ {}", f.path);
                }
            } else {
                println!("Package not installed: {}", pkg);
            }
            continue;
        }

        match db.remove_package(pkg) {
            Ok(file_paths) => {
                // Remove files from filesystem
                for file_path in &file_paths {
                    if let Err(e) = std::fs::remove_file(file_path) {
                        eprintln!("  Warning: Could not remove {}: {}", file_path, e);
                    }
                }
                println!("✓ {} removed ({} files)", pkg, file_paths.len());
            }
            Err(e) => {
                eprintln!("✗ Failed to remove {}: {}", pkg, e);
            }
        }
    }

    Ok(())
}

/// List all installed packages
fn list_packages() -> Result<()> {
    let db = StateDb::open().context("Failed to open state database")?;
    let packages = db.list_packages().context("Failed to list packages")?;

    if packages.is_empty() {
        println!("📋 No packages installed.");
    } else {
        println!("📋 Installed packages:");
        for pkg in packages {
            let installed = chrono_lite_format(pkg.installed_at);
            println!("  {} {} (installed {})", pkg.name, pkg.version, installed);
        }
    }

    Ok(())
}

/// Show info about a specific package
fn show_package_info(package: &str) -> Result<()> {
    let db = StateDb::open().context("Failed to open state database")?;

    match db.get_package(package).context("Failed to query database")? {
        Some(pkg) => {
            println!("📦 {}", pkg.name);
            println!("   Version: {}", pkg.version);
            println!("   Hash: {}", pkg.blake3);
            println!("   Installed: {}", chrono_lite_format(pkg.installed_at));

            let files = db.get_package_files(package)?;
            if !files.is_empty() {
                println!("   Files:");
                for f in files {
                    println!("     {}", f.path);
                }
            }
        }
        None => {
            println!("ℹ️  Package '{}' not found.", package);
        }
    }

    Ok(())
}

/// Simple timestamp formatter (avoids chrono dependency)
fn chrono_lite_format(unix_timestamp: i64) -> String {
    // Just show relative time for simplicity
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    
    let diff = now - unix_timestamp;
    
    if diff < 60 {
        "just now".to_string()
    } else if diff < 3600 {
        format!("{} minutes ago", diff / 60)
    } else if diff < 86400 {
        format!("{} hours ago", diff / 3600)
    } else {
        format!("{} days ago", diff / 86400)
    }
}

/// Compute BLAKE3 hash of files
fn hash_files(files: &[PathBuf]) -> Result<()> {
    for path in files {
        if !path.exists() {
            eprintln!("✗ File not found: {}", path.display());
            continue;
        }

        let data = std::fs::read(path)
            .context(format!("Failed to read {}", path.display()))?;
        
        let hash = blake3::hash(&data).to_hex();
        
        let filename = path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("(unknown)");
        
        println!("{} {}", hash, filename);
    }
    Ok(())
}

/// Garbage collect orphaned CAS blobs
fn garbage_collect(dry_run: bool) -> Result<()> {
    use std::collections::HashSet;
    
    let db = StateDb::open().context("Failed to open state database")?;
    let cas_path = dl::cas_path();
    
    // Get all hashes referenced by installed packages
    let packages = db.list_packages()?;
    let mut referenced_hashes: HashSet<String> = HashSet::new();
    
    for pkg in &packages {
        let files = db.get_package_files(&pkg.name)?;
        for f in files {
            referenced_hashes.insert(f.blake3);
        }
    }
    
    // Walk CAS directory and find orphans
    let mut orphans: Vec<(PathBuf, u64)> = Vec::new();
    let mut total_size: u64 = 0;
    
    if cas_path.exists() {
        for prefix_entry in std::fs::read_dir(&cas_path)? {
            let prefix_dir = prefix_entry?;
            if !prefix_dir.file_type()?.is_dir() { continue; }
            
            for blob_entry in std::fs::read_dir(prefix_dir.path())? {
                let blob = blob_entry?;
                let blob_name = blob.file_name().to_string_lossy().to_string();
                
                if !referenced_hashes.contains(&blob_name) {
                    let size = blob.metadata()?.len();
                    orphans.push((blob.path(), size));
                    total_size += size;
                }
            }
        }
    }
    
    if orphans.is_empty() {
        println!("✓ No orphaned blobs found.");
        return Ok(());
    }
    
    let size_mb = total_size as f64 / 1024.0 / 1024.0;
    
    if dry_run {
        println!("Would remove {} orphaned blobs ({:.2} MB):", orphans.len(), size_mb);
        for (path, size) in &orphans {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            println!("  {} ({} bytes)", &name[..16.min(name.len())], size);
        }
    } else {
        for (path, _) in &orphans {
            std::fs::remove_file(path)?;
        }
        println!("✓ Removed {} orphaned blobs ({:.2} MB)", orphans.len(), size_mb);
    }
    
    Ok(())
}

/// Search packages in the local index
fn search_packages(query: &str) -> Result<()> {
    use dl::index::PackageIndex;
    
    let index_path = dl_home().join("index.msgpack");
    if !index_path.exists() {
        bail!("No index found. Run 'dl update' first.");
    }
    
    let index = PackageIndex::load(&index_path)
        .context("Failed to load index")?;
    
    let results = index.search(query);
    
    if results.is_empty() {
        println!("No packages found matching '{}'", query);
    } else {
        println!("📦 Packages matching '{}':", query);
        for pkg in results {
            let desc = if pkg.description.is_empty() { "" } else { &pkg.description };
            println!("  {} {} — {}", pkg.name, pkg.version, desc);
        }
    }
    
    Ok(())
}

/// Generate index.msgpack from formulas directory
fn generate_index(formulas_dir: &std::path::Path, output: &std::path::Path) -> Result<()> {
    use dl::index::{PackageIndex, IndexEntry, IndexBottle};
    use std::time::{SystemTime, UNIX_EPOCH};
    
    if !formulas_dir.exists() {
        bail!("Formulas directory not found: {}", formulas_dir.display());
    }
    
    let mut index = PackageIndex::new();
    index.updated_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    
    let mut count = 0;
    for entry in std::fs::read_dir(formulas_dir)? {
        let entry = entry?;
        let path = entry.path();
        
        if path.extension().map(|e| e == "toml").unwrap_or(false) {
            match Formula::from_file(&path) {
                Ok(formula) => {
                    // Convert HashMap<String, Bottle> to Vec<IndexBottle>
                    let bottles: Vec<IndexBottle> = formula.bottle.iter().map(|(arch, b)| {
                        IndexBottle {
                            arch: arch.clone(),
                            url: b.url.clone(),
                            blake3: b.blake3.clone(),
                        }
                    }).collect();
                    
                    index.upsert(IndexEntry {
                        name: formula.package.name.clone(),
                        version: formula.package.version.clone(),
                        description: formula.package.description.clone(),
                        bottles,
                        deps: formula.dependencies.runtime.clone(),
                        bin: formula.install.bin.clone(),
                    });
                    count += 1;
                    println!("  + {}", formula.package.name);
                }
                Err(e) => {
                    eprintln!("  ✗ {}: {}", path.display(), e);
                }
            }
        }
    }
    
    index.save(output)?;
    println!("✓ Generated {} with {} packages", output.display(), count);
    
    Ok(())
}

/// Update package index from CDN
async fn update_index(url: &str, dry_run: bool) -> Result<()> {
    use dl::index::PackageIndex;
    
    let index_path = dl_home().join("index.msgpack");
    
    if dry_run {
        println!("Would download index from: {}", url);
        println!("Would save to: {}", index_path.display());
        return Ok(());
    }
    
    println!("🔄 Updating package index...");
    
    let client = Client::new();
    let response = client.get(url).send().await
        .context("Failed to fetch index")?;
    
    if !response.status().is_success() {
        bail!("Failed to fetch index: HTTP {}", response.status());
    }
    
    let bytes = response.bytes().await?;
    
    // Verify it's valid msgpack
    let index = PackageIndex::from_bytes(&bytes)
        .context("Invalid index format")?;
    
    // Save to disk
    std::fs::write(&index_path, &bytes)?;
    
    println!("✓ Updated index: {} packages", index.packages.len());
    
    Ok(())
}

/// Upgrade installed packages to latest versions
async fn upgrade_packages(packages: &[String], dry_run: bool) -> Result<()> {
    use dl::index::PackageIndex;

    let db = StateDb::open().context("Failed to open database")?;
    let installed = db.list_packages()?;
    
    if installed.is_empty() {
        println!("No packages installed.");
        return Ok(());
    }

    let index_path = dl_home().join("index.msgpack");
    if !index_path.exists() {
        bail!("No index found. Run 'dl update' first.");
    }
    
    let index = PackageIndex::load(&index_path)?;
    
    // Filter to requested packages, or all if empty
    let to_check: Vec<_> = if packages.is_empty() {
        installed.iter().collect()
    } else {
        installed.iter().filter(|p| packages.contains(&p.name)).collect()
    };
    
    let mut upgrades = Vec::new();
    
    for pkg in &to_check {
        if let Some(entry) = index.find(&pkg.name) {
            if entry.version != pkg.version {
                upgrades.push((pkg.name.clone(), pkg.version.clone(), entry.version.clone()));
            }
        }
    }
    
    if upgrades.is_empty() {
        println!("✓ All packages are up to date.");
        return Ok(());
    }
    
    if dry_run {
        println!("Would upgrade {} packages:", upgrades.len());
        for (name, old, new) in &upgrades {
            println!("  {} {} → {}", name, old, new);
        }
        return Ok(());
    }
    
    println!("⬆️ Upgrading {} packages...", upgrades.len());
    
    // Remove old, install new
    let client = Client::new();
    let cas = Cas::new()?;
    
    for (name, old, new) in &upgrades {
        println!("  {} {} → {}", name, old, new);
        
        // Remove old version
        db.remove_package(name)?;
        
        // Install new version from index
        match prepare_download(&client, name, false, None).await {
            Ok(Some(prepared)) => {
                finalize_package(&cas, &db, &prepared, false)?;
            }
            Ok(None) => {}
            Err(e) => eprintln!("  ✗ Failed to upgrade {}: {}", name, e),
        }
    }
    
    println!("✓ Upgraded {} packages", upgrades.len());
    
    Ok(())
}

/// Generate shell completions
fn generate_completions(shell: clap_complete::Shell) {
    use clap::CommandFactory;
    use clap_complete::generate;
    
    let mut cmd = Cli::command();
    generate(shell, &mut cmd, "dl", &mut std::io::stdout());
}

/// Create a new formula template
fn formula_new(name: &str, output_dir: &std::path::Path) -> Result<()> {
    if !output_dir.exists() {
        std::fs::create_dir_all(output_dir)?;
    }
    
    let path = output_dir.join(format!("{}.toml", name));
    if path.exists() {
        bail!("Formula already exists: {}", path.display());
    }
    
    let template = format!(r#"[package]
name = "{name}"
version = "0.1.0"
description = "A short description of {name}"
homepage = "https://example.com"
license = "MIT"

[source]
url = "https://example.com/{name}-0.1.0.tar.gz"
blake3 = "0000000000000000000000000000000000000000000000000000000000000000"

[bottle.arm64]
url = "https://example.com/releases/download/v0.1.0/{name}-0.1.0-arm64.tar.gz"
blake3 = "0000000000000000000000000000000000000000000000000000000000000000"

[dependencies]
runtime = []

[install]
bin = ["{name}"]
"#);

    std::fs::write(&path, template)?;
    println!("✓ Created new formula: {}", path.display());
    println!("  Edit this file and run 'dl formula check {}' to validate.", path.display());
    
    Ok(())
}

/// Validate a formula file
fn formula_check(path: &std::path::Path) -> Result<()> {
    if !path.exists() {
        bail!("File not found: {}", path.display());
    }
    
    match Formula::from_file(path) {
        Ok(formula) => {
            println!("✓ Formula is valid: {} {}", formula.package.name, formula.package.version);
            
            // Basic validation
            if formula.package.name.is_empty() {
                println!("  ⚠️ Warning: Package name is empty");
            }
            if formula.bottle.is_empty() {
                println!("  ⚠️ Warning: No bottles defined (prebuilt binaries will not be available)");
            }
            if formula.install.bin.is_empty() && formula.install.lib.is_empty() && formula.install.include.is_empty() && formula.install.script.is_empty() {
                println!("  ⚠️ Warning: No installation steps defined (bin, lib, include, or script)");
            }
        }
        Err(e) => {
            bail!("✗ Formula is invalid: {}", e);
        }
    }
    
    Ok(())
}

/// Update dl itself to the latest version
async fn self_update_dl(dry_run: bool) -> Result<()> {
    use dl::index::PackageIndex;

    let index_path = dl_home().join("index.msgpack");
    if !index_path.exists() {
        bail!("No index found. Run 'dl update' first.");
    }

    let index = PackageIndex::load(&index_path)
        .context("Failed to load index")?;

    let entry = index.find("dl")
        .context("dl itself is not in the package index. Ensure it is part of the formulas.")?;

    let current_version = env!("CARGO_PKG_VERSION");
    if entry.version == current_version {
        println!("✓ dl is already up to date ({})", current_version);
        return Ok(());
    }

    println!("⬆️  New version available: {} → {}", current_version, entry.version);

    if dry_run {
        println!("Would update dl to version {}", entry.version);
        return Ok(());
    }

    // Use current install infrastructure
    let client = Client::new();
    let cas = Cas::new()?;
    let db = StateDb::open()?;

    println!("📦 Downloading dl {}...", entry.version);
    
    match prepare_download(&client, "dl", false, None).await {
        Ok(Some(prepared)) => {
            // Find the dl binary in the prepared package
            let current_exe = std::env::current_exe()?;
            
            // We'll rely on finalize_package to store it in CAS and link it to ~/.dl/bin/dl
            finalize_package(&cas, &db, &prepared, false)?;
            
            println!("✓ dl updated successfully to {}", entry.version);
            println!("  New binary is at: {}", bin_path().join("dl").display());
            
            // Optional: check if we're running from ~/.dl/bin/dl
            let target_bin = bin_path().join("dl");
            if current_exe == target_bin {
                println!("✓ Successfully updated the running binary.");
            } else {
                println!("💡 You are currently running dl from: {}", current_exe.display());
                println!("   The update was installed to: {}", target_bin.display());
            }
        }
        Ok(None) => bail!("Failed to download update"),
        Err(e) => bail!("Update failed: {}", e),
    }

    Ok(())
}

/// Bump a formula version and update hashes
async fn formula_bump(path: &std::path::Path, version: &str, url: &str) -> Result<()> {
    println!("🚀 Bumping {} to {}...", path.display(), version);
    
    // 1. Download to temp and hash
    let client = Client::new();
    let temp_dir = tempfile::tempdir()?;
    let temp_file = temp_dir.path().join("bottle");
    
    println!("📦 Downloading new bottle to compute hash...");
    dl::downloader::download_with_progress(&client, url, &temp_file).await
        .context("Download failed")?;
    
    let hash = compute_file_hash_streaming(&temp_file)?;
    println!("✓ Computed hash: {}", hash);
    
    // 2. Load and update TOML
    let mut formula = Formula::from_file(path)?;
    formula.package.version = version.to_string();
    
    // Update the bottle for the current arch
    let current_arch = if cfg!(target_arch = "aarch64") { "arm64" } else { "x86_64" };
    
    if let Some(bottle) = formula.bottle.get_mut(current_arch) {
        bottle.url = url.to_string();
        bottle.blake3 = hash;
        println!("✓ Updated bottle for {}", current_arch);
    } else {
        println!("⚠️  Warning: No bottle found for {}. Not updating bottle hash.", current_arch);
    }
    
    // 3. Save back to TOML
    let toml_string = toml::to_string_pretty(&formula)?;
    std::fs::write(path, toml_string)?;
    
    println!("✓ Successfully updated {}", path.display());
    Ok(())
}

/// Compute BLAKE3 hash of a file using streaming (memory efficient)
fn compute_file_hash_streaming(path: &std::path::Path) -> Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)
        .context(format!("Failed to open {}", path.display()))?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0u8; 8192];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 { break; }
        hasher.update(&buffer[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

/// Run a package transiently without global installation
async fn run_package(pkg_name: &str, args: &[String], _dry_run: bool) -> Result<()> {
    let client = reqwest::Client::new();
    let cas = Cas::new()?;

    println!("🚀 Preparing to run '{}'...", pkg_name);

    // 1. Resolve and download
    let prepared = prepare_download(&client, pkg_name, false, None).await?
        .context(format!("Could not find or download package '{}'", pkg_name))?;

    // 2. Extract and store in CAS (transiently: as in, we don't symlink to ~/.dl/bin)
    let extract_dir = prepared.download_path.parent().unwrap().join("extracted");
    let extracted = dl::extractor::extract_auto(&prepared.download_path, &extract_dir)?;
    
    // Identify the binary to run (first in bin_list or package name)
    let bin_name = prepared.bin_list.first().cloned().unwrap_or_else(|| prepared.name.clone());
    let mut bin_path_in_cas = None;

    let is_raw = extracted.len() == 1 && 
        dl::extractor::detect_format(&prepared.download_path) == dl::extractor::ArchiveFormat::RawBinary;

    for file in &extracted {
        let hash = cas.store_file(&file.absolute_path)?;
        let is_match = if is_raw {
            true // If it's a raw binary and there's only one file, it's the one we want
        } else {
            file.relative_path.to_string_lossy() == bin_name || file.relative_path.file_name().unwrap().to_string_lossy() == bin_name
        };

        if is_match {
            bin_path_in_cas = Some(cas.blob_path(&hash));
        }
    }

    let bin_path = bin_path_in_cas.context(format!("Could not find binary '{}' in package archive", bin_name))?;

    // 3. Ensure executable and run
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&bin_path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&bin_path, perms)?;
    }

    let mut child = std::process::Command::new(bin_path)
        .args(args)
        .spawn()
        .context("Failed to spawn process")?;

    let status = child.wait().context("Failed to wait for process")?;
    
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }

    Ok(())
}
