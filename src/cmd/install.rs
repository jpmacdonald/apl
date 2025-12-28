//! Install command

use anyhow::{Context, Result, bail};
use futures::future::join_all;
use reqwest::Client;
use std::path::PathBuf;

use apl::core::version::PackageSpec;
use apl::cas::Cas;
use apl::db::StateDb;
use apl::downloader::download_and_verify;
use apl::package::{Package as Formula, PackageType};
use apl::lockfile::Lockfile;
use apl::{bin_path, apl_home};
use apl::io::dmg;

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
    use apl::index::PackageIndex;

    let db = StateDb::open().context("Failed to open state database")?;

    // Load index for resolution
    let index_path = apl_home().join("index.bin");
    let index = if index_path.exists() {
        PackageIndex::load(&index_path).ok()
    } else {
        None
    };

    // Load lockfile if --locked
    let lockfile = if locked {
        let lock_path = std::env::current_dir()?.join("apl.lock");
        if !lock_path.exists() {
            bail!("--locked specified but apl.lock not found");
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
        apl::resolver::resolve_dependencies(&names, index_ref)?
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
                perform_ux_checks(name);
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
    use apl::index::PackageIndex;
    use apl::package::{PackageInfo, Source, Dependencies, InstallSpec, Hints, Binary};

    let formula_path = PathBuf::from(pkg_name);
    
    // Try loading as file first, then lockfile, then fallback to index
    let (binary_url, binary_hash, formula) = if formula_path.exists() {
        let formula = Formula::from_file(&formula_path)?;
        let bottle = formula.binary_for_current_arch()
             .context("No binary for current architecture")?;
        (
            bottle.url.clone(),
            bottle.blake3.clone(),
            formula,
        )
    } else {
        // Check lockfile FIRST for authority
        let locked_data = if let Some(lf) = lockfile {
             if let Some(locked) = lf.find(pkg_name) {
                 // If version requested and doesn't match lock, ignore lock (or error?)
                 // cargo install --locked errors if mismatch. We will just prefer locked if it matches or strict mode.
                 // For now: if version match or no version requested, use lock.
                 let version_match = match requested_version {
                     Some(v) => v == locked.version,
                     None => true,
                 };
                 
                 if version_match {
                      // Reconstruct formula from lock info (minimal)
                      let _current_arch = apl::arch::current();
                      if locked.url.is_empty() {
                          None // Old lockfile without URLs, fall back to index
                      } else {
                          Some((locked.url.clone(), locked.blake3.clone(), locked.version.clone()))
                      }
                 } else {
                     None
                 }
             } else {
                 None
             }
        } else {
            None
        };

        if let Some((url, hash, version)) = locked_data {
            // Reconstruct minimal formula
            let type_ = PackageType::Cli; // Lockfile doesn't store type yet, assume CLI for now (TODO: store in lock)
             let formula = Formula {
                package: PackageInfo {
                    name: pkg_name.to_string(),
                    version,
                    description: "(locked)".to_string(),
                    homepage: String::new(),
                    license: String::new(),
                    type_,
                },
                source: Source {
                    url: String::new(),
                    blake3: String::new(),
                    strip_components: 0,
                },
                binary: std::collections::HashMap::new(), // Not needed for install
                dependencies: Dependencies {
                    runtime: vec![], // TODO: Lockfile needs to store deps to be perfect
                    build: vec![],
                    optional: vec![],
                },
                install: InstallSpec {
                    bin: vec![pkg_name.to_string()], // Guess binary name same as package
                    lib: vec![],
                    include: vec![],
                    script: String::new(),
                    app: None,
                },
                hints: Hints { post_install: String::new() },
            };
            (url, hash, formula)
        } else {
            // FALLBACK TO INDEX
            let index_path = apl_home().join("index.bin");
            if !index_path.exists() {
                bail!("Package '{}' not found and no index.bin. Run 'dl update'.", pkg_name);
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
            
            let current_arch = apl::arch::current();
            let bin_artifact = release.bottles.iter()
                .find(|b| b.arch.contains(current_arch) || b.arch == current_arch)
                .context(format!("No binary for {} (v{}) on {}", pkg_name, release.version, current_arch))?;
                
            let mut binary_map = std::collections::HashMap::new();
            binary_map.insert(current_arch.to_string(), Binary {
                url: bin_artifact.url.clone(),
                blake3: bin_artifact.blake3.clone(),
                arch: current_arch.to_string(),
                macos: "14.0".to_string(),
            });
            
            let type_ = match entry.type_.as_str() {
                "app" => PackageType::App,
                _ => PackageType::Cli,
            };

            let formula = Formula {
                package: PackageInfo {
                    name: entry.name.clone(),
                    version: release.version.clone(),
                    description: entry.description.clone(),
                    homepage: String::new(),
                    license: String::new(),
                    type_,
                },
                source: Source {
                    url: String::new(),
                    blake3: String::new(),
                    strip_components: 0,
                },
                binary: binary_map,
                dependencies: Dependencies {
                    runtime: release.deps.clone(),
                    build: vec![],
                    optional: vec![],
                },
                install: InstallSpec {
                    bin: if release.bin.is_empty() { vec![entry.name.clone()] } else { release.bin.clone() },
                    lib: vec![],
                    include: vec![],
                    script: String::new(),
                    app: release.app.clone(),
                },
                hints: Hints { post_install: release.hints.clone() },
            };
            
            (bin_artifact.url.clone(), bin_artifact.blake3.clone(), formula)
        }
    };

    if let Some(lf) = lockfile {
        if let Some(locked) = lf.find(&formula.package.name) {
            if locked.version != formula.package.version {
                bail!("Locked to {} but index has {}", locked.version, formula.package.version);
            }
        }
    }

    if dry_run {
        println!("Would install: {} {}", formula.package.name, formula.package.version);
        println!("  Source: {}", binary_url);
        return Ok(None);
    }

    let temp_dir = tempfile::tempdir()?;
    let url_filename = binary_url.split('/').last().unwrap_or("download");
    let temp_file = temp_dir.path().join(url_filename);

    download_and_verify(client, &binary_url, &temp_file, &binary_hash)
        .await
        .context("Download failed")?;

    Ok(Some(PreparedPackage {
        name: formula.package.name.clone(),
        version: formula.package.version.clone(),
        download_path: temp_file,
        bin_list: formula.install.bin.clone(),
        formula: Some(formula),
        blake3: binary_hash.to_string(),
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
    
    // Dispatch if App
    let is_app = pkg.formula.as_ref().map(|f| f.package.type_ == PackageType::App).unwrap_or(false);
    if is_app {
        return finalize_app_package(cas, db, pkg, _dry_run);
    }

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
    let extracted = apl::extractor::extract_auto(&pkg.download_path, &extract_dir)?;

    println!("  📂 Extracted {} files", extracted.len());

    let mut files_to_record = Vec::new();

    let is_raw = extracted.len() == 1 && 
        apl::extractor::detect_format(&pkg.download_path) == apl::extractor::ArchiveFormat::RawBinary;

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

        for bin_spec in bins {
            let (src_name, target_name) = if bin_spec.contains(':') {
                let parts: Vec<&str> = bin_spec.split(':').collect();
                (parts[0], parts[1])
            } else {
                (bin_spec, bin_spec)
            };

            // Only process if this file matches the source name (relative to archive root)
            if file.relative_path.to_string_lossy() != src_name && file_name != src_name {
                continue;
            }

            let target = bin_path().join(target_name);
            if target.exists() { std::fs::remove_file(&target).ok(); }
            
            cas.link_to(&hash, &target)?;
            #[cfg(unix)] {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755))?;
            }
            println!("  → {}", target_name);
            
            let abs_path = target.to_string_lossy().to_string();
            files_to_record.push((abs_path, hash.clone()));
        }
    }

    // Record in DB: Track version, active=true, artifacts and files atomically
    db.install_complete_package(&pkg.name, &pkg.version, &pkg.blake3, &files_to_record, &files_to_record)?;
    
    // Record history
    db.add_history(&pkg.name, "install", version_from.as_deref(), Some(&pkg.version), true)?;

    println!("✓ {} installed", pkg.name);

    update_lockfile_if_exists();

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
    perform_ux_checks(bin_name);

    Ok(())
}

fn finalize_app_package(
    _cas: &Cas,
    db: &StateDb,
    pkg: &PreparedPackage,
    _dry_run: bool,
) -> Result<()> {
    let app_name = pkg.formula.as_ref()
        .and_then(|f| f.install.app.as_ref())
        .ok_or_else(|| anyhow::anyhow!("type='app' requires [install] app='Name.app'"))?;

    let version_from = db.get_package(&pkg.name)?.map(|p| p.version);
    if let Some(ref v) = version_from {
        if v != &pkg.version {
             println!("  (Updating {} from {} to {})", pkg.name, v, pkg.version);
        }
    }
    
    let applications_dir = dirs::home_dir().context("No home dir")?.join("Applications");
    let target_app_path = applications_dir.join(app_name);
    
    // Clean old app
    if target_app_path.exists() {
         std::fs::remove_dir_all(&target_app_path).ok();
    }
    
    // Clean previous install files (from DB)
    if let Ok(files) = db.get_package_files(&pkg.name) {
        for f in files {
            let p = std::path::Path::new(&f.path);
            if p.exists() {
                 if p.is_dir() { std::fs::remove_dir_all(p).ok(); }
                 else { std::fs::remove_file(p).ok(); }
            }
        }
    }
    
    let is_dmg = pkg.download_path.extension().map_or(false, |e| e == "dmg");
    
    if is_dmg {
         let mount = dmg::attach(&pkg.download_path)?;
         println!("  💿 Mounted DMG at {}", mount.path.display());
         
         let src_app = mount.path.join(app_name);
         if !src_app.exists() {
             bail!("{} not found in DMG at {}", app_name, src_app.display());
         }
         
         // Copy recursively using cp -r
         let status = std::process::Command::new("cp")
             .arg("-r")
             .arg(&src_app)
             .arg(&target_app_path)
             .status()?;
             
         if !status.success() {
             bail!("Failed to copy .app bundle");
         }
         // Detach happens on drop
    } else {
         let extract_dir = pkg.download_path.parent().unwrap().join("extracted_app");
         if extract_dir.exists() { std::fs::remove_dir_all(&extract_dir).ok(); }
         
         apl::extractor::extract_auto(&pkg.download_path, &extract_dir)?;
         
         // Find app_name (recursive search or direct?)
         // Assume app is inside extract_dir (possibly nested in 1 dir)
         let mut found_path = extract_dir.join(app_name);
         if !found_path.exists() {
             // Try one level deep
             let entries: Vec<_> = std::fs::read_dir(&extract_dir)?.flatten().collect();
             if entries.len() == 1 && entries[0].file_type()?.is_dir() {
                 found_path = entries[0].path().join(app_name);
             }
         }
         
         if found_path.exists() {
             std::fs::rename(found_path, &target_app_path)?;
         } else {
             bail!("{} not found in archive", app_name);
         }
    }
    
    // Quarantine removal
    let _ = std::process::Command::new("xattr")
        .args(&["-r", "-d", "com.apple.quarantine"])
        .arg(&target_app_path)
        .status();
        
    let files_to_record = vec![(target_app_path.to_string_lossy().to_string(), "APP_BUNDLE".to_string())];
    
    db.install_complete_package(&pkg.name, &pkg.version, &pkg.blake3, &files_to_record, &files_to_record)?;
    db.add_history(&pkg.name, "install", version_from.as_deref(), Some(&pkg.version), true)?;
    
    println!("✓ {} installed to ~/Applications/{}", pkg.name, app_name);
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

    update_lockfile_if_exists();
    perform_ux_checks(name);

    Ok(())
}

fn update_lockfile_if_exists() {
    if apl::lockfile::Lockfile::exists_default() {
        println!("⟳ Updating lockfile...");
        if let Err(e) = crate::cmd::lock::lock(false) {
             println!("⚠ Failed to update lockfile: {}", e);
        }
    }
}

fn perform_ux_checks(bin_name: &str) {
    // PATH check
    let path_env = std::env::var_os("PATH").unwrap_or_default();
    let bin_dir = bin_path();
    let is_in_path = std::env::split_paths(&path_env).any(|p| p == bin_dir);

    if !is_in_path {
        println!();
        println!("⚠ Warning: {} is not in your PATH.", bin_dir.display());
        println!("  To use installed binaries, add this to your shell profile (~/.zshrc, ~/.bashrc, etc):");
        println!("  export PATH=\"{}:$PATH\"", bin_dir.display());
    } else {
        // Shadowing check
        if let Ok(output) = std::process::Command::new("which").arg(bin_name).output() {
            let which_path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let expected = bin_path().join(bin_name);
            if !which_path.is_empty() && !which_path.ends_with(&expected.to_string_lossy().to_string()) {
                println!();
                println!("💡 Note: {} is shadowed by {}.", bin_name, which_path);
                println!("   Run 'hash -r' or restart your terminal to use the new binary.");
            }
        }
    }
}
