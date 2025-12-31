use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use reqwest::Client;

use crate::core::relinker::Relinker;
use crate::core::version::PackageSpec;
use crate::db::StateDb;
use crate::io::dmg;
use crate::package::{ArtifactFormat, InstallStrategy, Package, PackageInfo, PackageType};
use crate::ui::Reporter;
use crate::{apl_home, bin_path, ops::InstallError, store_path};

/// Intermediate package state post-download, pending installation commit.
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

/// # Implementation Note: Installation Task Graph
///
/// We separate the concept of "what the user asked for" from "what we need to do".
/// 1. `Download`: The package is missing or needs an upgrade.
/// 2. `Switch`: The version is already unpacked in `~/.apl/store`, just update symlinks.
/// 3. `AlreadyInstalled`: Identical version is already active. No-op (but we report it).
///
/// This separation allows us to be efficient (don't re-download) and concurrent (downloads happen in parallel).
enum InstallTask {
    Download(String, Option<String>),
    Switch(String, String),
    AlreadyInstalled(String, String),
}

async fn resolve_and_filter_packages<R: Reporter>(
    packages: &[String],
    index: Option<&crate::core::index::PackageIndex>,
    reporter: &R,
) -> Result<(Vec<String>, Vec<PackageSpec>), InstallError> {
    let specs: Vec<PackageSpec> = packages
        .iter()
        .map(|p| PackageSpec::parse(p))
        .collect::<anyhow::Result<Vec<_>>>()
        .map_err(|e| InstallError::Validation(e.to_string()))?;

    let mut valid_names = Vec::new();
    if let Some(index_ref) = index {
        for spec in &specs {
            if Path::new(&spec.name).exists() || index_ref.find(&spec.name).is_some() {
                valid_names.push(spec.name.clone());
            } else {
                reporter.failed(&spec.name, "", "Package not found in index");
            }
        }
    } else {
        valid_names = specs.iter().map(|s| s.name.clone()).collect();
    }

    if valid_names.is_empty() && !specs.is_empty() {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        return Err(InstallError::Validation(
            "No valid packages found to install".to_string(),
        ));
    }

    let (local_file_names, index_names): (Vec<String>, Vec<String>) =
        valid_names.into_iter().partition(|n| Path::new(n).exists());

    let mut resolved_names = if index_names.is_empty() {
        Vec::new()
    } else {
        let index_ref = index.ok_or_else(|| {
            InstallError::Validation("No index found. Run 'apl update' first.".to_string())
        })?;

        let mut resolved = crate::resolver::resolve_dependencies(&index_names, index_ref)
            .map_err(|e| InstallError::Other(e.to_string()))?;

        resolved.sort();
        resolved.dedup();
        resolved
    };

    for local in local_file_names {
        resolved_names.push(local);
    }

    Ok((resolved_names, specs))
}

fn plan_install_tasks(
    resolved_names: &[String],
    specs: &[PackageSpec],
    index: Option<&crate::core::index::PackageIndex>,
    db: &StateDb,
) -> Result<Vec<InstallTask>, InstallError> {
    let mut tasks = Vec::new();
    let mut processed_names = std::collections::HashSet::new();

    for name in resolved_names {
        if !processed_names.insert(name.clone()) {
            continue;
        }

        let requested_version = specs
            .iter()
            .find(|s| &s.name == name)
            .and_then(|s| s.version().map(|v| v.to_string()));

        let target_version = if let Some(index_ref) = index {
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

        if let Ok(Some(installed)) = db.get_package_version(name, &target_version) {
            if installed.active {
                tasks.push(InstallTask::AlreadyInstalled(name.clone(), target_version));
                continue;
            } else {
                tasks.push(InstallTask::Switch(name.clone(), target_version));
                continue;
            }
        }

        tasks.push(InstallTask::Download(name.clone(), Some(target_version)));
    }

    Ok(tasks)
}

/// Resolves, downloads, and installs a set of packages.
pub async fn install_packages<R: Reporter + Clone + 'static>(
    reporter: &R,
    packages: &[String],
    dry_run: bool,
    _verbose: bool,
) -> Result<(), InstallError> {
    use crate::core::index::PackageIndex;

    let db = StateDb::open().map_err(|e| InstallError::Io(std::io::Error::other(e)))?;

    let index_path = apl_home().join("index.bin");
    let index = if index_path.exists() {
        PackageIndex::load(&index_path).ok()
    } else {
        None
    };

    // Phase 1: Logic - Resolve Dependencies
    let (resolved_names, specs) =
        resolve_and_filter_packages(packages, index.as_ref(), reporter).await?;

    // Phase 2: Planning - Determine Actions
    let tasks = plan_install_tasks(&resolved_names, &specs, index.as_ref(), &db)?;

    if tasks.is_empty() {
        return Ok(());
    }

    let table_items: Vec<(String, Option<String>)> = tasks
        .iter()
        .map(|t| match t {
            InstallTask::Download(n, v) => (n.clone(), v.clone()),
            InstallTask::AlreadyInstalled(n, v) => (n.clone(), Some(v.clone())),
            InstallTask::Switch(n, v) => (n.clone(), Some(v.clone())),
        })
        .collect();

    reporter.prepare_pipeline(&table_items);

    let client = Client::builder()
        .tcp_nodelay(true)
        .pool_max_idle_per_host(20)
        .build()
        .map_err(|e| InstallError::Download(crate::io::download::DownloadError::Http(e)))?;

    let start_time = Instant::now();
    let install_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let index_arc = Arc::new(index);
    let db_arc = Arc::new(Mutex::new(db));

    let mut already_installed_count = 0;
    for task in &tasks {
        match task {
            InstallTask::AlreadyInstalled(name, version) => {
                let size = if !dry_run {
                    db_arc
                        .lock()
                        .map_err(|_| InstallError::Lock("StateDb poisoned".to_string()))?
                        .get_package_version(name, version)
                        .ok()
                        .flatten()
                        .map(|p| p.size_bytes)
                } else {
                    None
                };
                reporter.done(name, version, "installed", size);
                already_installed_count += 1;
            }
            InstallTask::Switch(name, version) => {
                reporter.installing(name, version);
                if !dry_run {
                    crate::ops::switch::switch_version(name, version, dry_run, reporter)
                        .map_err(|e| InstallError::Other(e.to_string()))?;
                    install_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                } else {
                    reporter.done(name, version, "(dry run)", None);
                }
            }
            _ => {}
        }
    }

    let to_download: Vec<(String, Option<String>)> = tasks
        .iter()
        .filter_map(|t| match t {
            InstallTask::Download(n, v) => Some((n.clone(), v.clone())),
            _ => None,
        })
        .collect();

    if !to_download.is_empty() {
        let mut set: tokio::task::JoinSet<Result<Option<String>, InstallError>> =
            tokio::task::JoinSet::new();

        for (name, version) in to_download {
            let client = client.clone();
            let index = index_arc.clone();
            let reporter = reporter.clone();
            let db_arc = db_arc.clone();
            let install_count = install_count.clone();

            // Implementation Note: Parallel Downloads
            //
            // We use `JoinSet` here to run downloads concurrently. The concurrency limit
            // is implicitly controlled by the reqwest connection pool (set to 20 earlier).
            // Each task is independent; they don't share state except via the DB (which is locked)
            // and the filesystem (which they write to unique temp dirs).
            set.spawn(async move {
                let pkg_opt = prepare_download(
                    &client,
                    &name,
                    version.as_deref(),
                    index.as_ref().as_ref(),
                    &reporter,
                )
                .await?;

                if let Some(pkg) = pkg_opt {
                    if dry_run {
                        reporter.done(&name, &pkg.version, "installed", None);
                        return Ok(None);
                    }

                    reporter.installing(&name, &pkg.version);

                    let info = tokio::task::spawn_blocking(move || perform_local_install(pkg))
                        .await
                        .map_err(|e| InstallError::Other(format!("Task panic: {e}")))??;

                    let result = {
                        let db = db_arc
                            .lock()
                            .map_err(|_| InstallError::Lock("StateDb poisoned".to_string()))?;
                        commit_installation(&db, &info, &reporter)
                    };

                    if result.is_ok() {
                        reporter.done(
                            &name,
                            &info.package.version,
                            "installed",
                            Some(info.size_bytes),
                        );
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

        while let Some(res) = set.join_next().await {
            match res {
                Ok(Ok(Some(_))) => {}
                Ok(Ok(None)) => {}
                Ok(Err(e)) => reporter.error(&format!("Install failed: {e}")),
                Err(e) => reporter.error(&format!("Internal error: {e}")),
            }
        }
    }

    let count = install_count.load(std::sync::atomic::Ordering::Relaxed);
    if count > 0 {
        reporter.summary(count, "installed", start_time.elapsed().as_secs_f64());
    } else if already_installed_count > 0 {
        reporter.summary_plain(already_installed_count, "already installed");
    }

    let all_installed: Vec<String> = tasks
        .iter()
        .map(|t| match t {
            InstallTask::Download(n, _)
            | InstallTask::Switch(n, _)
            | InstallTask::AlreadyInstalled(n, _) => n.clone(),
        })
        .collect();

    perform_ux_checks(&all_installed, reporter);

    Ok(())
}

/// Resolves version requirements and retrieves the matching binary or source artifact.
pub async fn prepare_download<R: Reporter + Clone + 'static>(
    client: &Client,
    pkg_name: &str,
    requested_version: Option<&str>,
    index: Option<&crate::core::index::PackageIndex>,
    reporter: &R,
) -> Result<Option<PreparedPackage>, InstallError> {
    use crate::package::{Binary, Dependencies, Hints, InstallSpec, Source};

    let package_path = Path::new(pkg_name);

    let (binary_url, binary_hash, package_def, is_source) = if package_path.exists() {
        let package_def = Package::from_file(package_path)
            .map_err(|e| InstallError::Validation(e.to_string()))?;
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
            return Err(InstallError::Validation(format!(
                "Package {pkg_name} has no binary for this arch and no source."
            )));
        }
    } else {
        let index_ref = index.ok_or_else(|| {
            InstallError::Validation(format!("Index missing, cannot find {pkg_name}"))
        })?;
        let entry = index_ref
            .find(pkg_name)
            .ok_or_else(|| InstallError::Validation(format!("Package {pkg_name} not found")))?;

        let release = if let Some(v) = requested_version {
            if v == "latest" {
                entry.latest()
            } else {
                entry
                    .find_version(v)
                    .ok_or_else(|| InstallError::Validation(format!("Version {v} not found")))?
            }
        } else {
            entry.latest()
        };

        let current_arch = crate::arch::current();
        let bin_artifact = release
            .binaries
            .iter()
            .find(|b| b.arch.contains(current_arch) || b.arch == current_arch);

        let (url, hash, is_source) = if let Some(b) = bin_artifact {
            (b.url.clone(), b.blake3.clone(), false)
        } else if let Some(src) = &release.source {
            (src.url.clone(), src.blake3.clone(), true)
        } else {
            return Err(InstallError::Validation(format!(
                "No binary/source available for {pkg_name} on {current_arch}"
            )));
        };

        let mut binary_map = std::collections::HashMap::new();
        if !is_source {
            binary_map.insert(
                current_arch.to_string(),
                Binary {
                    url: url.clone(),
                    blake3: hash.clone(),
                    format: ArtifactFormat::Binary,
                    arch: current_arch.to_string(),
                    macos: "11.0".to_string(),
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
                type_: if entry.type_ == "app" {
                    PackageType::App
                } else {
                    PackageType::Cli
                },
            },
            source: Source {
                url: if is_source {
                    url.clone()
                } else {
                    String::new()
                },
                blake3: if is_source {
                    hash.clone()
                } else {
                    String::new()
                },
                format: ArtifactFormat::TarGz,
                strip_components: 1,
            },
            binary: binary_map,
            dependencies: Dependencies {
                runtime: release.deps.clone(),
                build: release.build_deps.clone(),
                optional: vec![],
            },
            install: InstallSpec {
                strategy: if entry.type_ == "app" {
                    InstallStrategy::App
                } else {
                    InstallStrategy::Link
                },
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
            build: if is_source {
                Some(crate::package::BuildSpec {
                    dependencies: release.build_deps.clone(),
                    script: release.build_script.clone(),
                })
            } else {
                None
            },
        };
        (url, hash, package_def, is_source)
    };

    let tmp_path = crate::tmp_path();
    std::fs::create_dir_all(&tmp_path).map_err(InstallError::Io)?;
    let temp_dir = tempfile::Builder::new()
        .prefix("apl-")
        .tempdir_in(tmp_path)
        .map_err(InstallError::Io)?;

    let pkg_format = if is_source {
        package_def.source.format.clone()
    } else {
        package_def
            .binary_for_current_arch()
            .map(|b| b.format.clone())
            .ok_or_else(|| InstallError::Validation("No binary format found".to_string()))?
    };

    let strategy = package_def.install.strategy.clone();
    let is_dmg = (strategy == InstallStrategy::App || strategy == InstallStrategy::Pkg)
        && (pkg_format == ArtifactFormat::Dmg
            || binary_url.to_lowercase().ends_with(".dmg")
            || binary_url.to_lowercase().ends_with(".pkg"));

    let download_or_extract_path: PathBuf;

    if is_dmg {
        let dest_file = temp_dir
            .path()
            .join(binary_url.split('/').last().unwrap_or("pkg.dmg"));
        crate::io::download::download_and_verify_mp(
            client,
            pkg_name,
            &package_def.package.version,
            &binary_url,
            &dest_file,
            &binary_hash,
            reporter,
        )
        .await?;
        download_or_extract_path = dest_file;
    } else {
        let cache_file = crate::cache_path().join(&binary_hash);
        if let Some(p) = cache_file.parent() {
            std::fs::create_dir_all(p).ok();
        }

        let extract_dir = temp_dir.path().join("extracted");
        std::fs::create_dir_all(&extract_dir).map_err(InstallError::Io)?;

        crate::io::download::download_and_extract(
            client,
            pkg_name,
            &package_def.package.version,
            &binary_url,
            &cache_file,
            &extract_dir,
            &binary_hash,
            reporter,
        )
        .await?;

        download_or_extract_path = extract_dir;
        if is_source && package_def.source.strip_components > 0 {
            crate::io::extract::strip_components(&download_or_extract_path)
                .map_err(|e| InstallError::Other(e.to_string()))?;
        }
    }

    Ok(Some(PreparedPackage {
        name: package_def.package.name.clone(),
        version: package_def.package.version.clone(),
        extracted_path: download_or_extract_path,
        bin_list: package_def.install.bin.clone(),
        package_def: Some(package_def),
        blake3: binary_hash,
        build_required: is_source,
        _temp_dir: Some(temp_dir),
    }))
}

struct InstallInfo {
    package: PackageInfo,
    blake3: String,
    files_to_record: Vec<(String, String)>,
    size_bytes: u64,
}

/// Moves artifacts to the final store location and updates symlinks.
fn perform_local_install(pkg: PreparedPackage) -> Result<InstallInfo, InstallError> {
    let package_def = pkg
        .package_def
        .as_ref()
        .ok_or_else(|| InstallError::Validation("Missing package definition".to_string()))?;
    let strategy = package_def.install.strategy.clone();

    let is_app = strategy == InstallStrategy::App
        || pkg
            .extracted_path
            .to_string_lossy()
            .to_lowercase()
            .ends_with(".dmg");

    if is_app || strategy == InstallStrategy::Pkg {
        return perform_app_install(pkg);
    }

    let pkg_store_path = store_path().join(&pkg.name).join(&pkg.version);
    if pkg_store_path.exists() {
        std::fs::remove_dir_all(&pkg_store_path).map_err(InstallError::Io)?;
    }
    std::fs::create_dir_all(pkg_store_path.parent().unwrap()).map_err(InstallError::Io)?;

    if pkg.build_required {
        perform_source_build(&pkg, &pkg_store_path, package_def)?;
    } else {
        std::fs::rename(&pkg.extracted_path, &pkg_store_path).map_err(|_| {
            InstallError::Other("Cross-volume move failed. APL requires store to be on the same volume as temp dir.".to_string())
        })?;
    }

    if !pkg.build_required {
        let _ = crate::io::extract::strip_components(&pkg_store_path);
    }

    relink_macho_files(&pkg_store_path);

    let mut files_to_record = Vec::new();
    let mut bins_to_link = Vec::new();

    if !package_def.install.bin.is_empty() {
        for bin_spec in &package_def.install.bin {
            if bin_spec.contains(':') {
                let parts: Vec<&str> = bin_spec.split(':').collect();
                bins_to_link.push((parts[0].to_string(), parts[1].to_string()));
            } else {
                let target = Path::new(bin_spec)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| bin_spec.clone());
                bins_to_link.push((bin_spec.clone(), target));
            }
        }
    } else {
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
                    if meta.is_file() {
                        use std::os::unix::fs::PermissionsExt;
                        if meta.permissions().mode() & 0o111 != 0 {
                            let name = entry.file_name().to_string_lossy().to_string();
                            bins_to_link.push((name.clone(), name));
                        }
                    }
                }
            }
        }
    }

    for (src_rel, target_name) in bins_to_link {
        let src_path = pkg_store_path.join(&src_rel);
        if !src_path.exists() {
            continue;
        }

        let target = bin_path().join(target_name);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        if target.exists() || target.is_symlink() {
            std::fs::remove_file(&target).ok();
        }

        #[cfg(unix)]
        std::os::unix::fs::symlink(&src_path, &target).map_err(InstallError::Io)?;

        files_to_record.push((target.to_string_lossy().to_string(), "SYMLINK".to_string()));
    }

    let size_bytes = calculate_dir_size(&pkg_store_path);

    Ok(InstallInfo {
        package: package_def.package.clone(),
        blake3: pkg.blake3,
        files_to_record,
        size_bytes,
    })
}

fn perform_source_build(
    pkg: &PreparedPackage,
    store_path: &Path,
    def: &Package,
) -> Result<(), InstallError> {
    let sysroot =
        crate::core::sysroot::Sysroot::new().map_err(|e| InstallError::Other(e.to_string()))?;
    let builder = crate::core::builder::Builder::new(&sysroot);
    let build_spec = def.build.as_ref().ok_or_else(|| {
        InstallError::Validation("Source build requires [build] section".to_string())
    })?;

    let missing = check_build_deps(&build_spec.dependencies);
    if !missing.is_empty() {
        return Err(InstallError::Validation(format!(
            "Missing build dependencies: {}",
            missing.join(", ")
        )));
    }

    let log_path = crate::build_log_path(&pkg.name, &pkg.version);
    builder
        .build(
            &pkg.extracted_path,
            &build_spec.script,
            store_path,
            false,
            &log_path,
        )
        .map_err(|e| InstallError::Script(e.to_string()))
}

fn perform_app_install(pkg: PreparedPackage) -> Result<InstallInfo, InstallError> {
    let app_name = pkg
        .package_def
        .as_ref()
        .and_then(|f| f.install.app.as_ref())
        .ok_or_else(|| {
            InstallError::Validation("type='app' requires [install] app='Name.app'".to_string())
        })?;

    let applications_dir = dirs::home_dir()
        .map(|h| h.join("Applications"))
        .unwrap_or_else(|| PathBuf::from("/Applications"));
    std::fs::create_dir_all(&applications_dir).map_err(InstallError::Io)?;

    let (_mount, search_path) = if pkg
        .extracted_path
        .to_string_lossy()
        .to_lowercase()
        .ends_with(".dmg")
    {
        let mount =
            dmg::attach(&pkg.extracted_path).map_err(|e| InstallError::Other(e.to_string()))?;
        let path = mount.path.clone();
        (Some(mount), path)
    } else {
        (None, pkg.extracted_path.clone())
    };

    let extracted_app = if search_path.extension().map_or(false, |e| e == "app") {
        search_path
    } else {
        let mut found = None;
        for entry in walkdir::WalkDir::new(&search_path)
            .min_depth(1)
            .max_depth(3)
            .into_iter()
            .flatten()
        {
            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            if entry.path().extension().map_or(false, |e| e == "app") {
                found = Some(entry.path().to_path_buf());
                break;
            }
        }
        found.ok_or_else(|| {
            InstallError::Validation(format!("No .app found in {}", search_path.display()))
        })?
    };

    let target_app = applications_dir.join(app_name);
    if target_app.exists() {
        std::fs::remove_dir_all(&target_app).map_err(InstallError::Io)?;
    }

    if let Err(_) = std::fs::rename(&extracted_app, &target_app) {
        crate::core::builder::copy_dir_all(&extracted_app, &target_app)
            .map_err(|e| InstallError::Other(e.to_string()))?;
    }

    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("xattr")
        .args(["-d", "com.apple.quarantine"])
        .arg(&target_app)
        .output();

    // Implementation Note: App Bundles
    //
    // macOS `.app` bundles are just directories. We must copy them to `/Applications`
    // instead of linking them. We also strip the "Quarantine" attribute so macOS
    // allows them to run (otherwise it complains they are from an unidentified developer).
    Ok(InstallInfo {
        package: pkg.package_def.unwrap().package,
        blake3: pkg.blake3,
        files_to_record: vec![(
            target_app.to_string_lossy().to_string(),
            "APP_BUNDLE".to_string(),
        )],
        size_bytes: 0,
    })
}

fn commit_installation(
    db: &StateDb,
    info: &InstallInfo,
    _reporter: &impl Reporter,
) -> Result<(), InstallError> {
    db.install_complete_package(
        &info.package.name,
        &info.package.version,
        &info.blake3,
        info.size_bytes,
        &[],
        &info.files_to_record,
    )
    .map_err(|e| InstallError::Other(e.to_string()))?;

    db.add_history(
        &info.package.name,
        "install",
        None,
        Some(&info.package.version),
        true,
    )
    .map_err(|e| InstallError::Other(e.to_string()))
}

pub fn perform_ux_checks(names: &[String], reporter: &impl Reporter) {
    let path_env = std::env::var_os("PATH").unwrap_or_default();
    let bin_dir = bin_path();
    let is_in_path = std::env::split_paths(&path_env).any(|p| p == bin_dir);

    if !is_in_path {
        reporter.warning(&format!("{} is not in your PATH.", bin_dir.display()));
        reporter.info(&format!(
            "Add this to your shell profile: export PATH=\"{}:$PATH\"",
            bin_dir.display()
        ));
    }

    for name in names {
        if let Ok(path) = which::which(name) {
            if !path.starts_with(&bin_dir) {
                reporter.warning(&format!(
                    "'{}' is shadowed by system version at {}",
                    name,
                    path.display()
                ));
            }
        }
    }
}

fn calculate_dir_size(path: &Path) -> u64 {
    walkdir::WalkDir::new(path)
        .into_iter()
        .flatten()
        .filter_map(|e| e.metadata().ok())
        .filter(|m| m.is_file())
        .map(|m| m.len())
        .sum()
}

fn relink_macho_files(path: &Path) {
    #[cfg(target_os = "macos")]
    for entry in walkdir::WalkDir::new(path).into_iter().flatten() {
        if entry.path().is_file() {
            let is_dylib = entry.path().extension().map_or(false, |e| e == "dylib");
            let is_exec = entry
                .metadata()
                .map(|m| {
                    use std::os::unix::fs::PermissionsExt;
                    m.permissions().mode() & 0o111 != 0
                })
                .unwrap_or(false);

            if is_dylib {
                let _ = Relinker::fix_dylib(entry.path());
            } else if is_exec {
                let _ = Relinker::fix_binary(entry.path());
            }
        }
    }
}

fn check_build_deps(deps: &[String]) -> Vec<String> {
    deps.iter()
        .filter(|d| which::which(d).is_err())
        .cloned()
        .collect()
}
