//! Install command

use anyhow::{Context, Result, bail};
use reqwest::Client;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use apl::cas::Cas;
use apl::core::version::PackageSpec;
use apl::db::StateDb;
use apl::io::dmg;
use apl::io::output::{InstallOutput, PackageProgress, format_size};
use apl::lockfile::Lockfile;
use apl::package::{Package as Formula, PackageInfo, PackageType};
use apl::{apl_home, bin_path};

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
pub async fn install(
    packages: &[String],
    dry_run: bool,
    locked: bool,
    _verbose: bool,
) -> Result<()> {
    use apl::index::PackageIndex;

    let db = StateDb::open().context("Failed to open state database")?;
    let progress = PackageProgress::new();
    let output = InstallOutput::new(false); // Legacy compatibility

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
    let specs: Vec<PackageSpec> = packages
        .iter()
        .map(|p| PackageSpec::parse(p))
        .collect::<Result<Vec<_>>>()?;

    // Validate existence in index before resolving
    // This allows us to fail gracefully for individual missing packages
    // while continuing to install the others.
    let mut valid_names = Vec::new();
    if let Some(index_ref) = &index {
        for spec in &specs {
            if index_ref.find(&spec.name).is_some() {
                valid_names.push(spec.name.clone());
            } else {
                // Print error immediately for missing packages
                // We fake a 'Fetching' header just for this error if it's the first thing?
                // Or we waits?
                // UI Semantics says:
                // Fetching ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
                //   ✗ foo                        Package 'foo' not found in index
                //   ✔ bar ...
                // So we should probably defer this printing or print it in the Fetching phase if we can.
                // However, resolution happens BEFORE Fetching.
                // We will print it cleanly here without the header, or trigger a failure that output recognizes?
                // Let's print it here. It might appear before "Fetching", which is acceptable/safer than crashing.
                output.fail(&spec.name, "", "Package not found in index");
            }
        }
    } else {
        // No index? All will likely fail in resolution if not local,
        // but we'll let resolver handle the "no index" error globally if it's strictly required.
        valid_names = specs.iter().map(|s| s.name.clone()).collect();
    }

    // Stop if nothing valid
    if valid_names.is_empty() {
        if !specs.is_empty() {
            return Ok(()); // All failed
        }
    }

    // Resolve dependencies for VALID packages only
    let resolved_names = {
        let index_ref = index
            .as_ref()
            .context("No index found. Run 'dl update' first.")?;

        let mut resolved = apl::resolver::resolve_dependencies(&valid_names, index_ref)?;

        // Ensure strictly unique list
        resolved.sort();
        resolved.dedup();
        resolved
    };

    let mut tasks = Vec::new();
    let mut processed_names = std::collections::HashSet::new();

    // Determine what to do for each resolved package
    for name in &resolved_names {
        if processed_names.contains(name) {
            continue;
        }
        processed_names.insert(name.clone());

        // Find if any spec explicitly requested this package (to get version)
        let requested_version = specs
            .iter()
            .find(|s| &s.name == name)
            .and_then(|s| s.version().map(|v| v.to_string()));

        // Determine target version from index (or latest)
        let target_version = if let Some(index_ref) = &index {
            if let Some(entry) = index_ref.find(name) {
                match &requested_version {
                    Some(v) if v == "latest" => entry.latest().version.clone(),
                    Some(v) => v.clone(),
                    None => entry.latest().version.clone(),
                }
            } else {
                requested_version
                    .clone()
                    .unwrap_or_else(|| "latest".to_string())
            }
        } else {
            requested_version
                .clone()
                .unwrap_or_else(|| "latest".to_string())
        };

        // Check DB for this specific version
        if let Ok(Some(installed)) = db.get_package_version(name, &target_version) {
            if installed.active {
                output.done(name, &target_version, "already installed");
                perform_ux_batch_checks(&[name.clone()], &output);
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
    // Local packages to install (e.g. from file paths)
    // let mut install_queued: Vec<PreparedPackage> = Vec::new();
    for task in &tasks {
        if let InstallTask::Switch(name, version) = task {
            finalize_switch(&cas, &db, name, version, dry_run, &output)?;
        }
    }

    // Wrap DB in Arc<Mutex> for sharing across tasks
    let db_arc = Arc::new(Mutex::new(db));

    // Process Downloads (parallel)
    let to_download: Vec<(String, Option<String>)> = tasks
        .iter()
        .filter_map(|t| match t {
            InstallTask::Download(n, v) => Some((n.clone(), v.clone())),
            _ => None,
        })
        .collect();

    // Pipelined Processing: Download -> Install
    if !to_download.is_empty() {
        output.section("Installing");

        // Pre-allocate progress bars for all packages
        let progress = Arc::new(progress);
        for (name, version) in &to_download {
            let ver = version.as_deref().unwrap_or("");
            progress.add_package(name, ver);
        }

        let client = Client::builder()
            .tcp_nodelay(true)
            .pool_max_idle_per_host(20)
            .build()?;

        let start_time = Instant::now();
        let install_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));

        let cas_arc = Arc::new(cas);
        let lockfile_arc = Arc::new(lockfile);
        let index_arc = Arc::new(index);

        let mut set: tokio::task::JoinSet<Result<Option<String>>> = tokio::task::JoinSet::new();

        for (name, version) in to_download.clone() {
            let client = client.clone();
            let lockfile = lockfile_arc.clone();
            let index = index_arc.clone();
            let progress = progress.clone();
            let cas = cas_arc.clone();
            let db_arc = db_arc.clone();
            let install_count = install_count.clone();

            set.spawn(async move {
                // 1. Download (bar updates bytes in-place)
                let pkg_opt = prepare_download_new(
                    &client,
                    &name,
                    version.as_deref(),
                    dry_run,
                    lockfile.as_ref().as_ref(),
                    index.as_ref().as_ref(),
                    &progress,
                )
                .await?;

                if let Some(pkg) = pkg_opt {
                    if dry_run {
                        progress.set_done(&pkg.name, &pkg.version);
                        return Ok(None);
                    }

                    // 2. Install (transition state)
                    progress.set_installing(&pkg.name, &pkg.version);

                    let info =
                        tokio::task::spawn_blocking(move || perform_local_install(pkg, &cas))
                            .await??;

                    // 3. Commit (Lock DB)
                    let result = {
                        let db = db_arc.lock().unwrap();
                        commit_installation_new(&*db, &info, &progress)
                    };

                    if result.is_ok() {
                        install_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        Ok(Some(info.package.name))
                    } else {
                        Ok(None)
                    }
                } else {
                    Ok(None)
                }
            });
        }

        // Wait for all tasks
        let mut installed_names = Vec::new();
        while let Some(res) = set.join_next().await {
            match res {
                Ok(Ok(Some(name))) => installed_names.push(name),
                Ok(Ok(None)) => {}
                Ok(Err(e)) => eprintln!("  ✘ Task failed: {}", e),
                Err(e) => eprintln!("  ✘ Task panicked: {}", e),
            }
        }

        // Print summary (safe to println after all bars finish)
        if !installed_names.is_empty() {
            check_path_shadowing(&installed_names);
            progress.print_summary(
                install_count.load(std::sync::atomic::Ordering::Relaxed),
                "installed",
                start_time.elapsed().as_secs_f64(),
            );
        }
    }

    update_lockfile_if_exists_quietly();

    Ok(())
}

/// Prepare a package download (updated with MultiProgress and Output)
pub async fn prepare_download_mp(
    client: &Client,
    pkg_name: &str,
    requested_version: Option<&str>,
    _dry_run: bool,
    lockfile: Option<&Lockfile>,
    index: Option<&apl::index::PackageIndex>,
    output: &InstallOutput,
) -> Result<Option<PreparedPackage>> {
    use apl::package::{Binary, Dependencies, Hints, InstallSpec, PackageInfo, Source};

    let formula_path = PathBuf::from(pkg_name);

    // Resolution logic (mostly the same, just quieted)
    let (binary_url, binary_hash, formula) = if formula_path.exists() {
        let formula = Formula::from_file(&formula_path)?;
        let bottle = formula
            .binary_for_current_arch()
            .context("No binary for current architecture")?;
        (bottle.url.clone(), bottle.blake3.clone(), formula)
    } else {
        // ... (rest of resolution logic, reused but made quiet)
        let locked_data = if let Some(lf) = lockfile {
            if let Some(locked) = lf.find(pkg_name) {
                let version_match = match requested_version {
                    Some(v) => v == locked.version,
                    None => true,
                };
                if version_match && !locked.url.is_empty() {
                    Some((
                        locked.url.clone(),
                        locked.blake3.clone(),
                        locked.version.clone(),
                    ))
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
            let formula = Formula {
                package: PackageInfo {
                    name: pkg_name.to_string(),
                    version,
                    description: String::new(),
                    homepage: String::new(),
                    license: String::new(),
                    type_: PackageType::Cli,
                },
                source: Source {
                    url: String::new(),
                    blake3: String::new(),
                    strip_components: 0,
                },
                binary: std::collections::HashMap::new(),
                dependencies: Dependencies {
                    runtime: vec![],
                    build: vec![],
                    optional: vec![],
                },
                install: InstallSpec {
                    bin: vec![pkg_name.to_string()],
                    lib: vec![],
                    include: vec![],
                    script: String::new(),
                    app: None,
                },
                hints: Hints {
                    post_install: String::new(),
                },
            };
            (url, hash, formula)
        } else {
            let index_ref = index.context(format!(
                "Package '{}' not found and no index found.",
                pkg_name
            ))?;
            let entry = index_ref
                .find(pkg_name)
                .context(format!("Package '{}' not found in index", pkg_name))?;
            let release = if let Some(v) = requested_version {
                if v == "latest" {
                    entry.latest()
                } else {
                    entry
                        .find_version(v)
                        .context(format!("Version '{}' not found", v))?
                }
            } else {
                entry.latest()
            };

            let current_arch = apl::arch::current();
            let bin_artifact = release
                .bottles
                .iter()
                .find(|b| b.arch.contains(current_arch) || b.arch == current_arch)
                .context(format!("No binary for {} on {}", pkg_name, current_arch))?;

            let mut binary_map = std::collections::HashMap::new();
            binary_map.insert(
                current_arch.to_string(),
                Binary {
                    url: bin_artifact.url.clone(),
                    blake3: bin_artifact.blake3.clone(),
                    arch: current_arch.to_string(),
                    macos: "14.0".to_string(),
                },
            );

            let formula = Formula {
                package: PackageInfo {
                    name: entry.name.clone(),
                    version: release.version.clone(),
                    description: entry.description.clone(),
                    homepage: String::new(),
                    license: String::new(),
                    type_: match entry.type_.as_str() {
                        "app" => PackageType::App,
                        _ => PackageType::Cli,
                    },
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
                    bin: if release.bin.is_empty() {
                        vec![entry.name.clone()]
                    } else {
                        release.bin.clone()
                    },
                    lib: vec![],
                    include: vec![],
                    script: String::new(),
                    app: release.app.clone(),
                },
                hints: Hints {
                    post_install: release.hints.clone(),
                },
            };
            (
                bin_artifact.url.clone(),
                bin_artifact.blake3.clone(),
                formula,
            )
        }
    };

    let temp_dir = tempfile::tempdir()?;
    let url_filename = binary_url.split('/').last().unwrap_or("download");
    let temp_file = temp_dir.path().join(url_filename);

    // Use InstallOutput for legacy compatibility
    let pb = output.download_bar(&formula.package.name, &formula.package.version, 0);

    // Download with verification
    let result = apl::io::download::download_and_verify_mp(
        client,
        &binary_url,
        &temp_file,
        &binary_hash,
        &pb,
    )
    .await;

    // Update to done/fail
    match result {
        Ok(_) => {
            let size = std::fs::metadata(&temp_file).map(|m| m.len()).unwrap_or(0);
            let size_str = format_size(size);
            output.finish_ok(&formula.package.name, &formula.package.version, &size_str);
        }
        Err(e) => {
            output.finish_err(
                &formula.package.name,
                &formula.package.version,
                &e.to_string(),
            );
            return Err(e.into());
        }
    }

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

/// Prepare a package download (new PackageProgress API)
async fn prepare_download_new(
    client: &Client,
    pkg_name: &str,
    requested_version: Option<&str>,
    _dry_run: bool,
    lockfile: Option<&Lockfile>,
    index: Option<&apl::index::PackageIndex>,
    progress: &PackageProgress,
) -> Result<Option<PreparedPackage>> {
    use apl::package::{Binary, Dependencies, Hints, InstallSpec, PackageInfo, Source};

    let formula_path = PathBuf::from(pkg_name);

    // Resolution logic
    let (binary_url, binary_hash, formula) = if formula_path.exists() {
        let formula = Formula::from_file(&formula_path)?;
        let bottle = formula
            .binary_for_current_arch()
            .context("No binary for current architecture")?;
        (bottle.url.clone(), bottle.blake3.clone(), formula)
    } else {
        let locked_data = if let Some(lf) = lockfile {
            if let Some(locked) = lf.find(pkg_name) {
                let version_match = match requested_version {
                    Some(v) => v == locked.version,
                    None => true,
                };
                if version_match && !locked.url.is_empty() {
                    Some((
                        locked.url.clone(),
                        locked.blake3.clone(),
                        locked.version.clone(),
                    ))
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
            let formula = Formula {
                package: PackageInfo {
                    name: pkg_name.to_string(),
                    version,
                    description: String::new(),
                    homepage: String::new(),
                    license: String::new(),
                    type_: PackageType::Cli,
                },
                source: Source {
                    url: String::new(),
                    blake3: String::new(),
                    strip_components: 0,
                },
                binary: std::collections::HashMap::new(),
                dependencies: Dependencies {
                    runtime: vec![],
                    build: vec![],
                    optional: vec![],
                },
                install: InstallSpec {
                    bin: vec![pkg_name.to_string()],
                    lib: vec![],
                    include: vec![],
                    script: String::new(),
                    app: None,
                },
                hints: Hints {
                    post_install: String::new(),
                },
            };
            (url, hash, formula)
        } else {
            let index_ref = index.context(format!(
                "Package '{}' not found and no index found.",
                pkg_name
            ))?;
            let entry = index_ref
                .find(pkg_name)
                .context(format!("Package '{}' not found in index", pkg_name))?;
            let release = if let Some(v) = requested_version {
                if v == "latest" {
                    entry.latest()
                } else {
                    entry
                        .find_version(v)
                        .context(format!("Version '{}' not found", v))?
                }
            } else {
                entry.latest()
            };

            let current_arch = apl::arch::current();
            let bin_artifact = release
                .bottles
                .iter()
                .find(|b| b.arch.contains(current_arch) || b.arch == current_arch)
                .context(format!("No binary for {} on {}", pkg_name, current_arch))?;

            let mut binary_map = std::collections::HashMap::new();
            binary_map.insert(
                current_arch.to_string(),
                Binary {
                    url: bin_artifact.url.clone(),
                    blake3: bin_artifact.blake3.clone(),
                    arch: current_arch.to_string(),
                    macos: "14.0".to_string(),
                },
            );

            let formula = Formula {
                package: PackageInfo {
                    name: entry.name.clone(),
                    version: release.version.clone(),
                    description: entry.description.clone(),
                    homepage: String::new(),
                    license: String::new(),
                    type_: match entry.type_.as_str() {
                        "app" => PackageType::App,
                        _ => PackageType::Cli,
                    },
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
                    bin: if release.bin.is_empty() {
                        vec![entry.name.clone()]
                    } else {
                        release.bin.clone()
                    },
                    lib: vec![],
                    include: vec![],
                    script: String::new(),
                    app: release.app.clone(),
                },
                hints: Hints {
                    post_install: release.hints.clone(),
                },
            };
            (
                bin_artifact.url.clone(),
                bin_artifact.blake3.clone(),
                formula,
            )
        }
    };

    let temp_dir = tempfile::tempdir()?;
    let url_filename = binary_url.split('/').last().unwrap_or("download");
    let temp_file = temp_dir.path().join(url_filename);

    // Get pre-allocated progress bar and download with live updates
    let pb = progress
        .get(pkg_name)
        .unwrap_or_else(|| progress.add_package(pkg_name, &formula.package.version));

    // Transition to downloading state (we don't know total size yet, but download_and_verify_mp will set it)
    progress.set_downloading(pkg_name, &formula.package.version, 0);

    let result = apl::io::download::download_and_verify_mp(
        client,
        &binary_url,
        &temp_file,
        &binary_hash,
        &pb,
    )
    .await;

    match result {
        Ok(_) => {
            // Download complete - stays as progress bar until set_installing is called
        }
        Err(e) => {
            progress.set_failed(pkg_name, &formula.package.version, &e.to_string());
            return Err(e.into());
        }
    }

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

/// Information about a successful local installation to be committed to DB
struct InstallInfo {
    package: PackageInfo,
    blake3: String,
    files_to_record: Vec<(String, String)>, // (path, hash)
}

/// Perform extraction and linking for a package (Thread Safe)
fn perform_local_install(pkg: PreparedPackage, cas: &Cas) -> Result<InstallInfo> {
    // Check if it's an .app bundle
    let is_app = pkg
        .formula
        .as_ref()
        .map(|f| f.package.type_ == PackageType::App)
        .unwrap_or(false)
        || pkg
            .download_path
            .to_string_lossy()
            .to_lowercase()
            .ends_with(".dmg");

    if is_app {
        return perform_app_install(pkg);
    }

    let mut files_to_record = Vec::new();

    let extract_dir = pkg.download_path.parent().unwrap().join("extracted");
    let extracted = apl::extractor::extract_auto(&pkg.download_path, &extract_dir)
        .map_err(|e| anyhow::anyhow!("Extraction failed: {}", e))?;

    let is_raw = extracted.len() == 1
        && apl::extractor::detect_format(&pkg.download_path)
            == apl::extractor::ArchiveFormat::RawBinary;

    for file in &extracted {
        let hash = cas
            .store_file(&file.absolute_path)
            .map_err(|e| anyhow::anyhow!("Failed to store in CAS: {}", e))?;

        let file_name = file
            .relative_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        let formula = pkg
            .formula
            .as_ref()
            .expect("Formula must be set in PreparedPackage");
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

            if file.relative_path.to_string_lossy() != src_name && file_name != src_name {
                continue;
            }

            let target = bin_path().join(target_name);
            if target.exists() {
                let _ = std::fs::remove_file(&target);
            }

            cas.link_to(&hash, &target)
                .map_err(|e| anyhow::anyhow!("Linking failed for {}: {}", target_name, e))?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755));
            }

            let abs_path = target.to_string_lossy().to_string();
            files_to_record.push((abs_path, hash.clone()));
        }
    }

    Ok(InstallInfo {
        package: pkg.formula.expect("Formula must be set").package,
        blake3: pkg.blake3,
        files_to_record,
    })
}

/// Perform app bundle installation (Thread Safe for FS, not DB)
fn perform_app_install(pkg: PreparedPackage) -> Result<InstallInfo> {
    let app_name = pkg
        .formula
        .as_ref()
        .and_then(|f| f.install.app.as_ref())
        .ok_or_else(|| anyhow::anyhow!("type='app' requires [install] app='Name.app'"))?;

    let applications_dir = dirs::home_dir()
        .context("No home dir")?
        .join("Applications");
    let target_app_path = applications_dir.join(app_name);

    // Clean old app
    if target_app_path.exists() {
        let _ = std::fs::remove_dir_all(&target_app_path);
    }

    let is_dmg = pkg.download_path.extension().map_or(false, |e| e == "dmg");

    if is_dmg {
        let mount = dmg::attach(&pkg.download_path)?;
        let src_app = mount.path.join(app_name);
        if !src_app.exists() {
            bail!("{} not found in DMG at {}", app_name, src_app.display());
        }

        let status = std::process::Command::new("cp")
            .arg("-r")
            .arg(&src_app)
            .arg(&target_app_path)
            .status()?;

        if !status.success() {
            bail!("Failed to copy .app bundle");
        }
    } else {
        let extract_dir = pkg.download_path.parent().unwrap().join("extracted_app");
        if extract_dir.exists() {
            let _ = std::fs::remove_dir_all(&extract_dir);
        }

        apl::extractor::extract_auto(&pkg.download_path, &extract_dir)
            .map_err(|e| anyhow::anyhow!("Extraction failed: {}", e))?;

        let mut found_path = extract_dir.join(app_name);
        if !found_path.exists() {
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

    let _ = std::process::Command::new("xattr")
        .args(&["-cr"])
        .arg(&target_app_path)
        .status();

    let files_to_record = vec![(
        target_app_path.to_string_lossy().to_string(),
        "APP_BUNDLE".to_string(),
    )];

    Ok(InstallInfo {
        package: pkg.formula.expect("Formula must be set").package,
        blake3: pkg.blake3,
        files_to_record,
    })
}

/// Finalize installation in the database
// This function is no longer used - replaced by commit_installation_new
/*
fn commit_installation(db: &StateDb, info: &InstallInfo, output: &InstallOutput) -> Result<()> {
    let pkg = &info.package;

    // Get version_from here (main thread)
    // Record status line is already handled by caller (real-time feedback)
    let version_from = db.get_package(&pkg.name).ok().flatten().map(|p| p.version);

    // Cleanup active file links (important: serialized)
    if let Ok(files) = db.get_package_files(&pkg.name) {
        for f in files {
            std::fs::remove_file(&f.path).ok();
        }
    }

    // Record in DB
    db.install_complete_package(
        &pkg.name,
        &pkg.version,
        &info.blake3,
        &info.files_to_record,
        &info.files_to_record,
    )
    .map_err(|e| anyhow::anyhow!("Database update failed: {}", e))?;

    // Record history
    db.add_history(
        &pkg.name,
        "install",
        version_from.as_deref(),
        Some(&pkg.version),
        true,
    )?;

    let status = "done".to_string();

    output.done(&pkg.name, &pkg.version, &status);

    if let Some(formula) = &pkg.formula {
        if !formula.hints.post_install.is_empty() {
            output.hint("Hint:");
            for line in formula.hints.post_install.lines() {
                output.hint(&format!("  {}", line));
            }
        }
    }

    Ok(())
}
*/

/// Finalize installation in the database (new PackageProgress API)
fn commit_installation_new(
    db: &StateDb,
    info: &InstallInfo,
    progress: &PackageProgress,
) -> Result<()> {
    let pkg = &info.package;

    let version_from = db.get_package(&pkg.name).ok().flatten().map(|p| p.version);

    // Cleanup active file links
    if let Ok(files) = db.get_package_files(&pkg.name) {
        for f in files {
            std::fs::remove_file(&f.path).ok();
        }
    }

    // Record in DB
    db.install_complete_package(
        &pkg.name,
        &pkg.version,
        &info.blake3,
        &info.files_to_record,
        &info.files_to_record,
    )
    .map_err(|e| anyhow::anyhow!("Database update failed: {}", e))?;

    // Record history
    db.add_history(
        &pkg.name,
        "install",
        version_from.as_deref(),
        Some(&pkg.version),
        true,
    )?;

    // Transition to "done"
    progress.set_done(&pkg.name, &pkg.version);

    Ok(())
}

/// Check if installed binaries are shadowed by other PATH entries
fn check_path_shadowing(installed_names: &[String]) {
    let apl_bin = bin_path();
    for name in installed_names {
        if let Ok(output) = std::process::Command::new("which").arg(name).output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.starts_with(apl_bin.to_string_lossy().as_ref()) {
                    eprintln!("  ⚠ {} is shadowed by {}", name, path);
                }
            }
        }
    }
}

/// Finalize a Switch operation (activates already installed version)
pub fn finalize_switch(
    cas: &Cas,
    db: &StateDb,
    name: &str,
    version: &str,
    dry_run: bool,
    output: &InstallOutput,
) -> Result<()> {
    if dry_run {
        output.done(name, version, "(dry run: switch)");
        return Ok(());
    }

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
        output.warn(&format!(
            "No artifacts found for {} {}. Reinstallation recommended.",
            name, version
        ));
    }

    let mut files_to_record = Vec::new();

    // 3. Link new artifacts
    for art in &artifacts {
        let target = std::path::Path::new(&art.path);
        // Ensure path is safe? (it's absolute from DB)
        if target.exists() {
            std::fs::remove_file(&target).ok();
        }

        cas.link_to(&art.blake3, target)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(target, std::fs::Permissions::from_mode(0o755))?;
        }
        output.verbose(&format!(
            "linked {}",
            target.file_name().unwrap().to_string_lossy()
        ));

        files_to_record.push((art.path.clone(), art.blake3.clone()));
    }

    // 4. Update DB status (active=true)
    let pkg_info = db
        .get_package_version(name, version)?
        .expect("Package disappeared from DB");

    db.install_package(name, version, &pkg_info.blake3)?; // active=true
    for (path, hash) in &files_to_record {
        db.add_file(path, name, hash)?;
    }

    // Record History
    db.add_history(name, "switch", version_from.as_deref(), Some(version), true)?;

    output.done(name, version, "done");

    perform_ux_batch_checks(&[name.to_string()], output);

    Ok(())
}

fn update_lockfile_if_exists_quietly() {
    if apl::lockfile::Lockfile::exists_default() {
        let _ = crate::cmd::lock::lock(false, true); // dry_run=false, silent=true
    }
}

fn perform_ux_batch_checks(names: &[String], output: &InstallOutput) {
    // 1. PATH check (global)
    let path_env = std::env::var_os("PATH").unwrap_or_default();
    let bin_dir = bin_path();
    let is_in_path = std::env::split_paths(&path_env).any(|p| p == bin_dir);

    if !is_in_path {
        output.warn(&format!("{} is not in your PATH.", bin_dir.display()));
        output.hint(
            "To use installed binaries, add this to your shell profile (~/.zshrc, ~/.bashrc, etc):",
        );
        output.hint(&format!("  export PATH=\"{}:$PATH\"", bin_dir.display()));
    }

    // 2. Shadowing check (per-package)
    for bin_name in names {
        if let Ok(output_cmd) = std::process::Command::new("which").arg(bin_name).output() {
            let which_path = String::from_utf8_lossy(&output_cmd.stdout)
                .trim()
                .to_string();
            let expected = bin_path().join(bin_name);
            if !which_path.is_empty()
                && !which_path.ends_with(&expected.to_string_lossy().to_string())
            {
                output.hint(&format!("{} is shadowed by {}.", bin_name, which_path));
                output.hint("   Run 'hash -r' or restart your terminal to use the new binary.");
            }
        }
    }
}
