//! Install command

use anyhow::{Context, Result, bail};
use reqwest::Client;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

// use apl::cas::Cas;
use apl::core::relinker::Relinker;
use apl::core::version::PackageSpec;
use apl::db::StateDb;
use apl::io::dmg;
use apl::lockfile::Lockfile;
use apl::package::{Package, PackageInfo, PackageType};
use apl::ui::Output;
use apl::{apl_home, bin_path, store_path};

/// Prepared package ready for finalization
pub struct PreparedPackage {
    pub name: String,
    pub version: String,
    pub extracted_path: PathBuf,
    pub package_def: Option<Package>,
    pub bin_list: Vec<String>,
    pub blake3: String,
    pub build_required: bool,
    pub _temp_dir: Option<tempfile::TempDir>,
}

enum InstallTask {
    Download(String, Option<String>), // name, requested_version
    Switch(String, String),           // name, target_version
    AlreadyInstalled(String, String), // name, version
}

/// Install one or more packages (parallel downloads, sequential install)
pub async fn install(
    packages: &[String],
    dry_run: bool,
    locked: bool,
    _verbose: bool,
) -> Result<()> {
    use apl::index::PackageIndex;

    let output = Output::new();
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
    let specs: Vec<PackageSpec> = packages
        .iter()
        .map(|p| PackageSpec::parse(p))
        .collect::<Result<Vec<_>>>()?;

    // Validate existence in index before resolving
    // EXCEPT for local file paths (*.toml) which bypass the index
    let mut valid_names = Vec::new();
    if let Some(index_ref) = &index {
        for spec in &specs {
            // Local file paths or packages in index are valid
            if std::path::Path::new(&spec.name).exists() || index_ref.find(&spec.name).is_some() {
                valid_names.push(spec.name.clone());
            } else {
                output.failed(&spec.name, "", "Package not found in index");
            }
        }
    } else {
        valid_names = specs.iter().map(|s| s.name.clone()).collect();
    }

    // Stop if nothing valid (but failures above will still show)
    if valid_names.is_empty() && !specs.is_empty() {
        // Give indicatif a moment to render failures
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        return Ok(());
    }

    // Resolve dependencies for VALID packages only
    // Local file paths bypass the resolver entirely
    let (local_file_names, index_names): (Vec<String>, Vec<String>) = valid_names
        .into_iter()
        .partition(|n| std::path::Path::new(n).exists());

    let mut resolved_names = if index_names.is_empty() {
        Vec::new()
    } else {
        let index_ref = index
            .as_ref()
            .context("No index found. Run 'dl update' first.")?;

        let mut resolved = apl::resolver::resolve_dependencies(&index_names, index_ref)?;

        // Ensure strictly unique list
        resolved.sort();
        resolved.dedup();
        resolved
    };

    // Prepend local file paths to resolved names
    for local in local_file_names {
        resolved_names.push(local);
    }

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
                tasks.push(InstallTask::AlreadyInstalled(name.clone(), target_version));
                continue;
            } else {
                // It's installed (inactive), so we Switch
                tasks.push(InstallTask::Switch(name.clone(), target_version));
                continue;
            }
        }

        // Not installed (or version mismatch), so Download
        tasks.push(InstallTask::Download(name.clone(), Some(target_version)));
    }

    if tasks.is_empty() {
        return Ok(());
    }

    // Prepare unified pipeline list
    let mut table_items: Vec<(String, Option<String>)> = Vec::new();
    for task in &tasks {
        match task {
            InstallTask::Download(n, v) => table_items.push((n.clone(), v.clone())),
            InstallTask::AlreadyInstalled(n, v) => table_items.push((n.clone(), Some(v.clone()))),
            InstallTask::Switch(n, v) => table_items.push((n.clone(), Some(v.clone()))),
        }
    }

    // Unified Pipeline
    output.prepare_pipeline(&table_items);

    let client = Client::builder()
        .tcp_nodelay(true)
        .pool_max_idle_per_host(20)
        .build()?;

    let start_time = Instant::now();
    let install_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    // Cas logic removed
    // let cas = Cas::new()?;
    // let cas_arc = Arc::new(cas);
    let lockfile_arc = Arc::new(lockfile);
    let index_arc = Arc::new(index);
    let db_arc = Arc::new(Mutex::new(db));

    // Handle AlreadyInstalled and Switch tasks first (synchronously update UI)
    let mut already_installed_count = 0;
    for task in &tasks {
        match task {
            InstallTask::AlreadyInstalled(name, version) => {
                let size = if !dry_run {
                    let db_guard = db_arc
                        .lock()
                        .map_err(|_| anyhow::anyhow!("Database lock poisoned"))
                        .ok();
                    // We just ignore the error here since it's just for size display
                    db_guard
                        .and_then(|db| db.get_package_version(name, version).ok().flatten())
                        .map(|p| p.size_bytes)
                } else {
                    None
                };
                output.done(name, version, "installed", size);
                already_installed_count += 1;
            }
            InstallTask::Switch(name, version) => {
                output.installing(name, version);
                if !dry_run {
                    match crate::cmd::r#use::use_version(name, version, dry_run) {
                        Ok(_) => {
                            install_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        }
                        Err(e) => output.failed(name, version, &e.to_string()),
                    }
                } else {
                    output.done(name, version, "(dry run)", None);
                }
            }
            _ => {} // Downloads handled below
        }
    }

    // Process Downloads (parallel)
    let to_download: Vec<(String, Option<String>)> = tasks
        .iter()
        .filter_map(|t| match t {
            InstallTask::Download(n, v) => Some((n.clone(), v.clone())),
            _ => None,
        })
        .collect();

    if !to_download.is_empty() {
        let mut set: tokio::task::JoinSet<Result<Option<String>>> = tokio::task::JoinSet::new();

        for (name, version) in to_download.clone() {
            let client = client.clone();
            let lockfile = lockfile_arc.clone();
            let index = index_arc.clone();
            let output = output.clone();
            let db_arc = db_arc.clone();
            let install_count = install_count.clone();

            set.spawn(async move {
                // 1. Fetching (under "Fetching" section)
                let pkg_opt = prepare_download_new(
                    &client,
                    &name,
                    version.as_deref(),
                    dry_run,
                    lockfile.as_ref().as_ref(),
                    index.as_ref().as_ref(),
                    &output,
                )
                .await?;

                if let Some(pkg) = pkg_opt {
                    if dry_run {
                        output.done(&name, &pkg.version, "installed", None);
                        return Ok(None);
                    }

                    // 2. Installing
                    output.installing(&name, &pkg.version);

                    let info =
                        tokio::task::spawn_blocking(move || perform_local_install(pkg)).await??;

                    // 3. Commit (Lock DB)
                    let result = {
                        let db = db_arc
                            .lock()
                            .map_err(|_| anyhow::anyhow!("Database lock poisoned"))?;
                        commit_installation_new(&db, &info, &output)
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
        // Wait for all tasks
        while let Some(res) = set.join_next().await {
            match res {
                Ok(Ok(Some(_))) => {}
                Ok(Ok(None)) => {}
                Ok(Err(e)) => eprintln!("  ✘ Task failed: {e}"),
                Err(e) => eprintln!("  ✘ Task panicked: {e}"),
            }
        }
    }

    // Final Summary
    let count = install_count.load(std::sync::atomic::Ordering::Relaxed);
    if count > 0 {
        output.summary(count, "installed", start_time.elapsed().as_secs_f64());
        // Sync UI actor
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    } else if already_installed_count > 0 {
        output.summary_plain(already_installed_count, "already installed");
        // Sync UI actor
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }

    // Shadowing and path checks at the very end
    let all_installed: Vec<String> = tasks
        .iter()
        .map(|t| match t {
            InstallTask::Download(n, _) => n.clone(),
            InstallTask::Switch(n, _) => n.clone(),
            InstallTask::AlreadyInstalled(n, _) => n.clone(),
        })
        .collect();
    perform_ux_batch_checks(&all_installed, &output);

    update_lockfile_if_exists_quietly();

    Ok(())
}

/// Prepare a package download (new PackageProgress API)
pub async fn prepare_download_new(
    client: &Client,
    pkg_name: &str,
    requested_version: Option<&str>,
    _dry_run: bool,
    lockfile: Option<&Lockfile>,
    index: Option<&apl::index::PackageIndex>,
    output: &Output,
) -> Result<Option<PreparedPackage>> {
    use apl::package::{Binary, Dependencies, Hints, InstallSpec, PackageInfo, Source};

    let package_path = PathBuf::from(pkg_name);

    // Resolution logic
    let (binary_url, binary_hash, package_def, is_source) = if package_path.exists() {
        let package_def = Package::from_file(&package_path)?;
        // Try binary first, fallback to source
        if let Some(bottle) = package_def.binary_for_current_arch() {
            (
                bottle.url.clone(),
                bottle.blake3.clone(),
                package_def,
                false,
            )
        } else if !package_def.source.url.is_empty() {
            (
                package_def.source.url.clone(),
                package_def.source.blake3.clone(),
                package_def,
                true,
            )
        } else {
            anyhow::bail!("Package has no binary for current arch and no source defined.");
        }
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
            let package_def = Package {
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
                build: None,
            };
            (url, hash, package_def, false)
        } else {
            let index_ref = index.context(format!(
                "Package '{pkg_name}' not found and no index found."
            ))?;
            let entry = index_ref
                .find(pkg_name)
                .context(format!("Package '{pkg_name}' not found in index"))?;
            let release = if let Some(v) = requested_version {
                if v == "latest" {
                    entry.latest()
                } else {
                    entry
                        .find_version(v)
                        .context(format!("Version '{v}' not found"))?
                }
            } else {
                entry.latest()
            };

            let current_arch = apl::arch::current();
            let bin_artifact = release
                .binaries
                .iter()
                .find(|b| b.arch.contains(current_arch) || b.arch == current_arch);

            let (url, hash, is_source) = if let Some(b) = bin_artifact {
                (b.url.clone(), b.blake3.clone(), false)
            } else if let Some(src) = &release.source {
                (src.url.clone(), src.blake3.clone(), true)
            } else {
                anyhow::bail!(
                    "No binary for {pkg_name} on {current_arch} and no source available."
                );
            };

            let mut binary_map = std::collections::HashMap::new();
            if !is_source {
                binary_map.insert(
                    current_arch.to_string(),
                    Binary {
                        url: url.clone(),
                        blake3: hash.clone(),
                        arch: current_arch.to_string(),
                        macos: "14.0".to_string(),
                    },
                );
            }

            let package_def = Package {
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
                source: if is_source {
                    Source {
                        url: url.clone(),
                        blake3: hash.clone(),
                        strip_components: 1, // Assume standard tarballs
                    }
                } else {
                    Source {
                        url: String::new(),
                        blake3: String::new(),
                        strip_components: 0,
                    }
                },
                binary: binary_map,
                dependencies: Dependencies {
                    runtime: release.deps.clone(),
                    build: release.build_deps.clone(),
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
                    post_install: match release.hints.as_str() {
                        "" => String::new(),
                        s => s.to_string(),
                    },
                },
                build: if is_source {
                    Some(apl::package::BuildSpec {
                        dependencies: release.build_deps.clone(),
                        script: release.build_script.clone(),
                    })
                } else {
                    None
                },
            };
            (url, hash, package_def, is_source)
        }
    };

    let tmp_dir_path = apl::tmp_path();
    if !tmp_dir_path.exists() {
        std::fs::create_dir_all(&tmp_dir_path).ok();
    }

    let temp_dir = tempfile::Builder::new()
        .prefix("apl-dl-")
        .tempdir_in(tmp_dir_path)?;

    // Determine artifact filename from URL
    let url_filename = binary_url.split('/').next_back().unwrap_or("download");

    // Check type
    let is_app = package_def.package.type_ == PackageType::App;

    let download_or_extract_path: PathBuf;

    if is_app {
        // Legacy File Download for Apps/DMGs
        let dest_file = temp_dir.path().join(url_filename);
        // Removed: output.add_package(pkg_name, &package_def.package.version);

        match apl::io::download::download_and_verify_mp(
            client,
            pkg_name,
            &package_def.package.version,
            &binary_url,
            &dest_file,
            &binary_hash,
            output,
        )
        .await
        {
            Ok(_) => {}
            Err(e) => {
                output.failed(pkg_name, &package_def.package.version, &e.to_string());
                return Err(e.into());
            }
        }
        download_or_extract_path = dest_file;
    } else {
        // Pipelined Download & Extract for CLI
        let cache_file = apl::cache_path().join(&binary_hash);
        // Create cache dir if missing
        if let Some(p) = cache_file.parent() {
            tokio::fs::create_dir_all(p).await.ok();
        }

        let extract_dir = temp_dir.path().join("extracted");
        tokio::fs::create_dir_all(&extract_dir).await?;

        // Removed: output.add_package(pkg_name, &package_def.package.version);

        match apl::io::download::download_and_extract(
            client,
            pkg_name,
            &package_def.package.version,
            &binary_url,
            &cache_file,
            &extract_dir,
            &binary_hash,
            output,
        )
        .await
        {
            Ok(_) => {}
            Err(e) => {
                output.failed(pkg_name, &package_def.package.version, &e.to_string());
                return Err(e.into());
            }
        }
        download_or_extract_path = extract_dir;
    }

    Ok(Some(PreparedPackage {
        name: package_def.package.name.clone(),
        version: package_def.package.version.clone(),
        extracted_path: download_or_extract_path,
        bin_list: package_def.install.bin.clone(),
        package_def: Some(package_def),
        blake3: binary_hash.to_string(),
        build_required: is_source,
        _temp_dir: Some(temp_dir),
    }))
}

/// Information about a successful local installation to be committed to DB
struct InstallInfo {
    package: PackageInfo,
    blake3: String,
    files_to_record: Vec<(String, String)>, // (path, hash)
    size_bytes: u64,
}

/// Perform extraction and linking for a package (Thread Safe)
fn perform_local_install(pkg: PreparedPackage) -> Result<InstallInfo> {
    // Check if it's an .app bundle
    let package_def = pkg
        .package_def
        .as_ref()
        .expect("Package definition must be set in PreparedPackage");

    let is_app = (package_def.package.type_ == PackageType::App)
        || pkg
            .extracted_path
            .to_string_lossy()
            .to_lowercase()
            .ends_with(".dmg");

    if is_app {
        return perform_app_install(pkg);
    }

    let pkg_store_path = store_path().join(&pkg.name).join(&pkg.version);
    if pkg_store_path.exists() {
        let _ = std::fs::remove_dir_all(&pkg_store_path);
    }
    // We don't create_dir_all because rename will place the directory there
    // But rename expects the *parent* to exist? store_path() exists.
    std::fs::create_dir_all(pkg_store_path.parent().unwrap())?;

    // 1. Move Extracted Dir to Store (Atomic-ish) OR Build
    if pkg.build_required {
        // Build from Source
        let sysroot = apl::core::sysroot::Sysroot::new().context("Failed to create sysroot")?;
        let builder = apl::core::builder::Builder::new(&sysroot);

        let build_spec = package_def
            .build
            .as_ref()
            .context("Build specification missing for source build")?;

        // Check build dependencies are available
        let missing_deps = check_build_deps(&build_spec.dependencies);
        if !missing_deps.is_empty() {
            anyhow::bail!(
                "Missing build dependencies: {}. Install them first with: apl install {}",
                missing_deps.join(", "),
                missing_deps.join(" ")
            );
        }

        builder
            .build(&pkg.extracted_path, &build_spec.script, &pkg_store_path)
            .context("Source build failed")?;
    } else {
        // Binary Install: Rename extracted dir to store
        if let Err(_e) = std::fs::rename(&pkg.extracted_path, &pkg_store_path) {
            anyhow::bail!(
                "Failed to move extracted package to store (atomic move failed). Ensure strict volume co-location."
            );
        }
    }

    // 2. Strip Components (e.g. nvim-osx64/bin/... -> bin/...)
    if !pkg.build_required {
        let _ = apl::extractor::strip_components(&pkg_store_path);
    }

    // 2.5. Relink Mach-O binaries and dylibs for relocatability
    relink_macho_files(&pkg_store_path);

    // 3. Link Binaries
    let mut files_to_record = Vec::new();

    // Determine binaries to link
    let mut bins_to_link = Vec::new();
    if !package_def.install.bin.is_empty() {
        for bin_spec in &package_def.install.bin {
            let (src_name, target_name): (String, String) = if bin_spec.contains(':') {
                let parts: Vec<&str> = bin_spec.split(':').collect();
                (parts[0].to_string(), parts[1].to_string())
            } else {
                // Extract basename for target when no explicit target given
                let target = std::path::Path::new(bin_spec.as_str())
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| bin_spec.clone());
                (bin_spec.clone(), target)
            };
            bins_to_link.push((src_name, target_name));
        }
    } else {
        // Auto-detect executables in store/bin or root
        let bin_dir = pkg_store_path.join("bin");
        let search_dir = if bin_dir.exists() {
            &bin_dir
        } else {
            &pkg_store_path
        };
        if let Ok(entries) = std::fs::read_dir(search_dir) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        if meta.is_file() && (meta.permissions().mode() & 0o111 != 0) {
                            let name = entry.file_name().to_string_lossy().to_string();
                            bins_to_link.push((name.clone(), name));
                        }
                    }
                }
            }
        }
    }

    for (src_rel, target_name) in bins_to_link {
        // Find the source file in store
        let src_path = pkg_store_path.join(&src_rel);
        let src_path = if !src_path.exists() && pkg_store_path.join("bin").join(&src_rel).exists() {
            pkg_store_path.join("bin").join(&src_rel)
        } else {
            src_path
        };

        if !src_path.exists() {
            continue;
        }

        let target = bin_path().join(target_name);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        if target.exists() {
            let _ = std::fs::remove_file(&target);
        }

        // Use symlink for Store Model (modern/standard)
        #[cfg(unix)]
        std::os::unix::fs::symlink(&src_path, &target)
            .map_err(|e| anyhow::anyhow!("Symlinking failed: {e}"))?;

        let abs_path = target.to_string_lossy().to_string();
        // Record the symlink and the store path?
        // For now, let's record the link so 'remove' knows what to delete.
        files_to_record.push((abs_path, "SYMLINK".to_string()));
    }

    // 4. Calculate Size (Recursive Store)
    let mut size_bytes = 0;
    for entry in walkdir::WalkDir::new(&pkg_store_path).into_iter().flatten() {
        if let Ok(meta) = entry.metadata() {
            if meta.is_file() {
                size_bytes += meta.len();
            }
        }
    }

    Ok(InstallInfo {
        package: package_def.package.clone(),
        blake3: pkg.blake3,
        files_to_record,
        size_bytes,
    })
}

/// Perform app bundle installation (Thread Safe for FS, not DB)
fn perform_app_install(pkg: PreparedPackage) -> Result<InstallInfo> {
    let app_name = pkg
        .package_def
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

    let is_dmg = pkg.extracted_path.extension().is_some_and(|e| e == "dmg");

    if is_dmg {
        let mount = dmg::attach(&pkg.extracted_path)?;
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
        let extract_dir = pkg.extracted_path.parent().unwrap().join("extracted_app");
        if extract_dir.exists() {
            let _ = std::fs::remove_dir_all(&extract_dir);
        }

        apl::extractor::extract_auto(&pkg.extracted_path, &extract_dir)
            .map_err(|e| anyhow::anyhow!("Extraction failed: {e}"))?;

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
            bail!("{app_name} not found in archive");
        }
    }

    let _ = std::process::Command::new("xattr")
        .args(["-cr"])
        .arg(&target_app_path)
        .status();

    let files_to_record = vec![(
        target_app_path.to_string_lossy().to_string(),
        "APP_BUNDLE".to_string(),
    )];

    // Calculate size of app bundle
    let mut size_bytes = 0;
    if target_app_path.is_dir() {
        for entry in walkdir::WalkDir::new(&target_app_path)
            .into_iter()
            .flatten()
        {
            if let Ok(meta) = entry.metadata() {
                if meta.is_file() {
                    size_bytes += meta.len();
                }
            }
        }
    } else if let Ok(meta) = std::fs::metadata(&target_app_path) {
        size_bytes = meta.len();
    }

    Ok(InstallInfo {
        package: pkg
            .package_def
            .expect("Package definition must be set")
            .package,
        blake3: pkg.blake3,
        files_to_record,
        size_bytes,
    })
}

/// Finalize installation in the database (new PackageProgress API)
fn commit_installation_new(db: &StateDb, info: &InstallInfo, output: &Output) -> Result<()> {
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
        info.size_bytes,
        &info.files_to_record,
        &info.files_to_record,
    )
    .map_err(|e| anyhow::anyhow!("Database update failed: {e}"))?;

    // Record history
    db.add_history(
        &pkg.name,
        "install",
        version_from.as_deref(),
        Some(&pkg.version),
        true,
    )?;

    // Transition to "done"
    output.done(&pkg.name, &pkg.version, "installed", Some(info.size_bytes));

    Ok(())
}

// finalize_switch removed (legacy CAS logic)

fn update_lockfile_if_exists_quietly() {
    if apl::lockfile::Lockfile::exists_default() {
        let _ = crate::cmd::lock::lock(false, true); // dry_run=false, silent=true
    }
}

pub fn perform_ux_batch_checks(names: &[String], output: &Output) {
    // 1. PATH check (global)
    let path_env = std::env::var_os("PATH").unwrap_or_default();
    let bin_dir = bin_path();
    let is_in_path = std::env::split_paths(&path_env).any(|p| p == bin_dir);

    if !is_in_path {
        output.warning(&format!("{} is not in your PATH.", bin_dir.display()));
        output.info("To use installed binaries, add this to your shell profile:");
        println!("  export PATH=\"{}:$PATH\"", bin_dir.display());
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
                output.warning(&format!("{bin_name} is shadowed by {which_path}."));
                // (Info line removed per user request)
            }
        }
    }
}

/// Relink Mach-O binaries and dylibs in the given directory for relocatability
fn relink_macho_files(store_path: &std::path::Path) {
    // Walk bin/ and lib/ directories
    let bin_dir = store_path.join("bin");
    let lib_dir = store_path.join("lib");

    // Fix executables in bin/
    if bin_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&bin_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    // Attempt to add rpath - ignore errors (file might not be Mach-O)
                    let _ = Relinker::fix_binary(&path);
                }
            }
        }
    }

    // Fix dylibs in lib/
    if lib_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&lib_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "dylib") {
                    let _ = Relinker::fix_dylib(&path);
                }
            }
        }
    }
}

/// Check if build dependencies are available in PATH or APL store
/// Returns list of missing dependencies
fn check_build_deps(deps: &[String]) -> Vec<String> {
    let mut missing = Vec::new();
    let apl_bin = bin_path();

    for dep in deps {
        // Check if the tool is in PATH
        let in_path = std::process::Command::new("which")
            .arg(dep)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        // Check if installed in APL store
        let in_apl = apl_bin.join(dep).exists();

        if !in_path && !in_apl {
            missing.push(dep.clone());
        }
    }

    missing
}
