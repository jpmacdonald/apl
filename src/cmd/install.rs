//! Install command

use anyhow::{Context, Result, bail};
use futures::future::join_all;
use reqwest::Client;
use std::path::PathBuf;

use dl::core::version::PackageSpec;
use dl::cas::Cas;
use dl::db::StateDb;
use dl::downloader::download_and_verify;
use dl::formula::Formula;
use dl::lockfile::Lockfile;
use dl::{bin_path, dl_home};

/// Prepared package ready for finalization
pub struct PreparedPackage {
    pub name: String,
    pub version: String,
    pub download_path: PathBuf,
    pub formula: Option<Formula>,
    pub bin_list: Vec<String>,
    pub blake3: String, 
    pub _temp_dir: Option<tempfile::TempDir>,
}

enum InstallTask {
    Download(String, Option<String>), // name, requested_version
    Switch(String, String),           // name, target_version
}

/// Install one or more packages (parallel downloads, sequential install)
pub async fn install(packages: &[String], dry_run: bool, locked: bool) -> Result<()> {
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

    // Parse package specs for @version syntax
    let specs: Vec<PackageSpec> = packages.iter()
        .map(|p| PackageSpec::parse(p))
        .collect::<Result<Vec<_>>>()?;

    // Resolve dependencies
    let resolved_names = {
        let index_ref = index.as_ref()
            .context("No index found. Run 'dl update' first.")?;
        
        let names: Vec<String> = specs.iter().map(|s| s.name.clone()).collect();
        dl::resolver::resolve_dependencies(&names, index_ref)?
    };

    let mut tasks = Vec::new();

    // Determine what to do for each resolved package
    for name in &resolved_names {
        // Find if any spec explicitly requested this package (to get version)
        let requested_version = specs.iter()
            .find(|s| &s.name == name)
            .and_then(|s| s.version.clone());

        // Determine target version from index (or latest)
        let target_version = if let Some(index_ref) = &index {
            if let Some(entry) = index_ref.find(name) {
                 match &requested_version {
                    Some(v) if v == "latest" => entry.latest().version.clone(),
                    Some(v) => v.clone(),
                    None => entry.latest().version.clone(),
                }
            } else {
                 requested_version.clone().unwrap_or_else(|| "latest".to_string())
            }
        } else {
            requested_version.clone().unwrap_or_else(|| "latest".to_string())
        };
        
        // Check DB for this specific version
        if let Ok(Some(installed)) = db.get_package_version(name, &target_version) {
            if installed.active {
                println!("✓ {} {} already installed and active", name, target_version);
                continue; 
            } else {
                // It's installed (inactive), so we Switch
                tasks.push(InstallTask::Switch(name.clone(), target_version));
                continue;
            }
        }
        
        // Not installed (or version mismatch), so Download
        tasks.push(InstallTask::Download(name.clone(), requested_version));
    }

    if tasks.is_empty() {
        return Ok(());
    }

    // Process Switches first (fast)
    let cas = Cas::new()?;
    for task in &tasks {
        if let InstallTask::Switch(name, version) = task {
            finalize_switch(&cas, &db, name, version, dry_run)?;
        }
    }

    // Process Downloads (parallel)
    let to_download: Vec<_> = tasks.iter().filter_map(|t| match t {
        InstallTask::Download(n, v) => Some((n, v)),
        _ => None,
    }).collect();

    if !to_download.is_empty() {
        println!("📦 Downloading {} packages...", to_download.len());
        
        let client = Client::new();
        let download_futures: Vec<_> = to_download.iter()
            .map(|(name, version)| prepare_download(&client, name, version.as_deref(), dry_run, lockfile.as_ref()))
            .collect();

        let results = join_all(download_futures).await;

        let mut prepared = Vec::new();
        for result in results {
            match result {
                Ok(Some(pkg)) => prepared.push(pkg),
                Ok(None) => {} 
                Err(e) => return Err(e),
            }
        }

        if dry_run {
            return Ok(());
        }

        // Finalize downloads
        for pkg in &prepared {
            finalize_package(&cas, &db, pkg, dry_run)?;
        }
    }

    Ok(())
}

/// Prepare a package download
pub async fn prepare_download(
    client: &Client,
    pkg_name: &str,
    requested_version: Option<&str>,
    dry_run: bool,
    lockfile: Option<&Lockfile>,
) -> Result<Option<PreparedPackage>> {
    use dl::index::PackageIndex;

    let formula_path = PathBuf::from(pkg_name);
    
    // Try loading as file first, then fall back to index lookup
    let (name, version, bottle_url, bottle_hash, bin_list, hints_str, formula_opt) = if formula_path.exists() {
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
        let index_path = dl_home().join("index.bin");
        if !index_path.exists() {
            bail!("Package '{}' not found. Run 'dl update' to fetch package index.", pkg_name);
        }
        
        let index = PackageIndex::load(&index_path)
            .context("Failed to load index")?;
        
        let entry = index.find(pkg_name)
            .context(format!("Package '{}' not found in index", pkg_name))?;
        
        let release = if let Some(v) = requested_version {
            if v == "latest" {
                entry.latest()
            } else {
                entry.find_version(v)
                    .context(format!("Version '{}' of package '{}' not found in index", v, pkg_name))?
            }
        } else {
            entry.latest()
        };
        
        let current_arch = dl::arch::current();
        let bottle = release.bottles.iter()
            .find(|b| b.arch.contains(current_arch) || b.arch == current_arch)
            .context(format!("No bottle for {} (v{}) on {}", pkg_name, release.version, current_arch))?;
        
        (
            entry.name.clone(),
            release.version.clone(),
            bottle.url.clone(),
            bottle.blake3.clone(),
            release.bin.clone(),
            release.hints.clone(),
            None,
        )
    };

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

    let temp_dir = tempfile::tempdir()?;
    let url_filename = bottle_url.split('/').last().unwrap_or("download");
    let temp_file = temp_dir.path().join(url_filename);

    download_and_verify(client, &bottle_url, &temp_file, &bottle_hash)
        .await
        .context("Download failed")?;

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
        blake3: bottle_hash,
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

    let version_from = db.get_package(&pkg.name)?.map(|p| p.version);
    
    if let Some(ref v) = version_from {
        if v != &pkg.version {
             println!("  (Updating {} from {} to {})", pkg.name, v, pkg.version);
        }
    }

    // Cleanup active file links
    if let Ok(files) = db.get_package_files(&pkg.name) {
        for f in files {
            std::fs::remove_file(&f.path).ok();
        }
    }
    
    let extract_dir = pkg.download_path.parent().unwrap().join("extracted");
    let extracted = dl::extractor::extract_auto(&pkg.download_path, &extract_dir)?;

    println!("  📂 Extracted {} files", extracted.len());

    let mut files_to_record = Vec::new();

    let is_raw = extracted.len() == 1 && 
        dl::extractor::detect_format(&pkg.download_path) == dl::extractor::ArchiveFormat::RawBinary;

    for file in &extracted {
        let hash = cas.store_file(&file.absolute_path)?;

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
            
            let abs_path = target.to_string_lossy().to_string();
            files_to_record.push((abs_path, hash.clone()));
        }
    }

    // Record in DB: Track version, active=true, artifacts and files atomically
    db.install_complete_package(&pkg.name, &pkg.version, &pkg.blake3, &files_to_record, &files_to_record)?;
    
    // Record history
    db.add_history(&pkg.name, "install", version_from.as_deref(), Some(&pkg.version), true)?;

    println!("✓ {} installed", pkg.name);

    if let Some(formula) = &pkg.formula {
        if !formula.hints.post_install.is_empty() {
            println!();
            println!("💡 Hint:");
            for line in formula.hints.post_install.lines() {
                println!("   {}", line);
            }
        }
    }

    let bin_name = pkg.formula.as_ref()
        .and_then(|f| f.install.bin.first())
        .unwrap_or(&pkg.name);
    if let Ok(output) = std::process::Command::new("which").arg(bin_name).output() {
        let which_path = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let expected = bin_path().join(bin_name);
        if !which_path.is_empty() && which_path != expected.to_string_lossy() {
            println!();
            println!("💡 Run 'hash -r' or restart your terminal to use the new binary");
        }
    }

    Ok(())
}

/// Finalize a Switch operation (activates already installed version)
pub fn finalize_switch(
    cas: &Cas,
    db: &StateDb,
    name: &str,
    version: &str,
    dry_run: bool,
) -> Result<()> {
    if dry_run {
        println!("Would switch {} to {}", name, version);
        return Ok(());
    }
    
    println!("🔄 Switching {} to {}...", name, version);
    
    let version_from = db.get_package(name)?.map(|p| p.version);

    // 1. Unlink current files
     if let Ok(files) = db.get_package_files(name) {
        for f in files {
            std::fs::remove_file(&f.path).ok();
        }
    }
    
    // 2. Retrieve artifacts for target version
    let artifacts = db.get_artifacts(name, version)?;
    if artifacts.is_empty() {
        println!("⚠️ Warning: No artifacts found for {} {}. Reinstallation recommended.", name, version);
    }
    
    let mut files_to_record = Vec::new();
    
    // 3. Link new artifacts
    for art in &artifacts {
        let target = std::path::Path::new(&art.path);
        // Ensure path is safe? (it's absolute from DB)
        if target.exists() { std::fs::remove_file(&target).ok(); }
        
        cas.link_to(&art.blake3, target)?;
        #[cfg(unix)] {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(target, std::fs::Permissions::from_mode(0o755))?;
        }
        println!("  → {}", target.file_name().unwrap().to_string_lossy());
        
        files_to_record.push((art.path.clone(), art.blake3.clone()));
    }
    
    // 4. Update DB status (active=true)
    let pkg_info = db.get_package_version(name, version)?.expect("Package disappeared from DB");
    
    db.install_package(name, version, &pkg_info.blake3)?; // active=true
    for (path, hash) in &files_to_record {
        db.add_file(path, name, hash)?;
    }
    
    // Record History
    db.add_history(name, "switch", version_from.as_deref(), Some(version), true)?;
    
    println!("✓ active version is now {}", version);
    Ok(())
}
