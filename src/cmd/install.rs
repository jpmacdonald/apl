//! Install command

use anyhow::{Context, Result, bail};
use futures::future::join_all;
use reqwest::Client;
use std::path::PathBuf;

use crate::core::version::PackageSpec;
use dl::cas::Cas;
use dl::db::StateDb;
use dl::downloader::download_and_verify;
use dl::formula::Formula;
use dl::index::PackageIndex;
use dl::lockfile::Lockfile;
use dl::{bin_path, dl_home};

/// Prepared package ready for finalization
pub struct PreparedPackage {
    pub name: String,
    pub version: String,
    pub download_path: PathBuf,
    pub formula: Option<Formula>,
    pub bin_list: Vec<String>,
    pub _temp_dir: Option<tempfile::TempDir>,
}

/// Install one or more packages (parallel downloads, sequential install)
pub async fn install(packages: &[String], dry_run: bool, locked: bool) -> Result<()> {
    use dl::lockfile::Lockfile;
    use dl::index::PackageIndex;

    let db = StateDb::open().context("Failed to open state database")?;

    // Load index for resolution
    let index_path = dl_home().join("index.bin");
    let index = if index_path.exists() {
        PackageIndex::load(&index_path).ok()
    } else {
        None
    };

    // Load lockfile if --locked
    let lockfile = if locked {
        let lock_path = std::env::current_dir()?.join("dl.lock");
        if !lock_path.exists() {
            bail!("--locked specified but dl.lock not found");
        }
        Some(Lockfile::load(&lock_path)?)
    } else {
        None
    };

    // Resolve dependencies
    let resolved = {
        let index_ref = index.as_ref()
            .context("No index found. Run 'dl update' first.")?;
        
        // Parse package specs for @version syntax
        let specs: Vec<PackageSpec> = packages.iter()
            .map(|p| PackageSpec::parse(p))
            .collect::<Result<Vec<_>>>()?;
        
        let names: Vec<String> = specs.iter().map(|s| s.name.clone()).collect();
        dl::resolver::resolve_dependencies(&names, index_ref)?
    };

    println!("📦 Downloading {} packages...", resolved.len());

    // Phase 1: Download in parallel
    let client = Client::new();
    let download_futures: Vec<_> = resolved.iter()
        .map(|pkg| prepare_download(&client, pkg, dry_run, lockfile.as_ref()))
        .collect();

    let results = join_all(download_futures).await;

    // Collect successful downloads
    let mut prepared = Vec::new();
    for result in results {
        match result {
            Ok(Some(pkg)) => prepared.push(pkg),
            Ok(None) => {} // Dry run or already installed
            Err(e) => return Err(e),
        }
    }

    if dry_run {
        return Ok(());
    }

    // Phase 2: Install sequentially (DB writes)
    let cas = Cas::new()?;
    for pkg in &prepared {
        finalize_package(&cas, &db, pkg, dry_run)?;
    }

    Ok(())
}

/// Prepare a package download (parallel-safe, no DB access)
pub async fn prepare_download(
    client: &Client,
    pkg: &str,
    dry_run: bool,
    lockfile: Option<&Lockfile>,
) -> Result<Option<PreparedPackage>> {
    use dl::index::PackageIndex;

    let formula_path = PathBuf::from(pkg);
    
    // Try loading as file first, then fall back to index lookup
    let (name, version, bottle_url, bottle_hash, bin_list, hints_str, formula_opt) = if formula_path.exists() {
        // Load from formula file
        let formula = Formula::from_file(&formula_path)?;
        let bottle = formula.bottle_for_current_arch()
            .context("No bottle for current architecture")?;
        (
            formula.package.name.clone(),
            formula.package.version.clone(),
            bottle.url.clone(),
            bottle.blake3.clone(),
            formula.install.bin.clone(),
            formula.hints.post_install.clone(),
            Some(formula),
        )
    } else {
        // Try to find in index
        let index_path = dl_home().join("index.bin");
        if !index_path.exists() {
            bail!("Package '{}' not found. Run 'dl update' to fetch package index.", pkg);
        }
        
        let index = PackageIndex::load(&index_path)
            .context("Failed to load index")?;
        
        let entry = index.find(pkg)
            .context(format!("Package '{}' not found in index", pkg))?;
        
        // Find bottle for current arch
        let current_arch = dl::arch::current();
        let bottle = entry.bottles.iter()
            .find(|b| b.arch.contains(current_arch) || b.arch == current_arch)
            .context(format!("No bottle for {} on {}", pkg, current_arch))?;
        
        (
            entry.name.clone(),
            entry.version.clone(),
            bottle.url.clone(),
            bottle.blake3.clone(),
            entry.bin.clone(),
            entry.hints.clone(),
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
            hints: dl::formula::Hints { post_install: hints_str },
        }
    });

    Ok(Some(PreparedPackage {
        name: name.clone(),
        version: version.clone(),
        download_path: temp_file,
        formula: Some(formula),
        bin_list,
        _temp_dir: Some(temp_dir),
    }))
}

/// Finalize package installation (sequential, uses DB)
pub fn finalize_package(
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

    // Print hints if available
    if let Some(formula) = &pkg.formula {
        if !formula.hints.post_install.is_empty() {
            println!();
            println!("💡 Hint:");
            for line in formula.hints.post_install.lines() {
                println!("   {}", line);
            }
        }
    }

    Ok(())
}
