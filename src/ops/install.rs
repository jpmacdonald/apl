//! Package installation operations.
//!
//! This module provides the core installation logic for APL, including:
//!
//! - Resolving package names to specific versions
//! - Downloading and verifying artifacts
//! - Extracting archives and linking binaries
//! - Managing symlinks in `~/.apl/bin`
//!
//! The main entry point is [`install_packages`], which handles the full
//! installation workflow including dependency resolution and parallel downloads.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use reqwest::Client;

use crate::DbHandle;
use crate::core::relinker::Relinker;
use crate::core::version::PackageSpec;
use crate::io::dmg;
use crate::package::{InstallStrategy, Package, PackageInfo};
use crate::types::PackageName;
use crate::types::Version;
use crate::ui::Reporter;
use crate::{apl_home, bin_path, ops::InstallError, ops::link_binaries, store_path};

use crate::ops::flow::{PreparedPackage, UnresolvedPackage};

/// # Implementation Note: Installation Task Graph
///
/// We separate the concept of "what the user asked for" from "what we need to do".
/// 1. `Download`: The package is missing or needs an upgrade.
/// 2. `Switch`: The version is already unpacked in `~/.apl/store`, just update symlinks.
/// 3. `AlreadyInstalled`: Identical version is already active. No-op (but we report it).
///
/// This separation allows us to be efficient (don't re-download) and concurrent (downloads happen in parallel).
enum InstallTask {
    Download(PackageName, Option<Version>),
    Switch(PackageName, Version),
    AlreadyInstalled(PackageName, Version),
}

async fn resolve_and_filter_packages<R: Reporter>(
    packages: &[String],
    index: Option<&crate::core::index::PackageIndex>,
    reporter: &R,
) -> Result<(Vec<PackageName>, Vec<PackageSpec>), InstallError> {
    let specs: Vec<PackageSpec> = packages
        .iter()
        .map(|p| PackageSpec::parse(p))
        .collect::<anyhow::Result<Vec<_>>>()
        .map_err(|e| InstallError::Validation(e.to_string()))?;

    let mut valid_names: Vec<PackageName> = Vec::new();
    if let Some(index_ref) = index {
        for spec in &specs {
            if Path::new(&*spec.name).exists() || index_ref.find(&spec.name).is_some() {
                valid_names.push(spec.name.clone());
            } else {
                reporter.failed(&spec.name, &Version::from(""), "Package not found in index");
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

    let (local_file_names, index_names): (Vec<PackageName>, Vec<PackageName>) = valid_names
        .into_iter()
        .partition(|n| Path::new(&**n).exists());

    let mut resolved_names = if index_names.is_empty() {
        Vec::new()
    } else {
        let index_ref = index.ok_or_else(|| {
            InstallError::Validation("No index found. Run 'apl update' first.".to_string())
        })?;

        let mut resolved = crate::core::resolver::resolve_dependencies(&index_names, index_ref)
            .map_err(|e| InstallError::context("Dependency resolution failed", e))?;

        resolved.sort();
        resolved.dedup();
        resolved
    };

    for local in local_file_names {
        resolved_names.push(local);
    }

    Ok((resolved_names, specs))
}

async fn plan_install_tasks(
    resolved_names: &[PackageName],
    specs: &[PackageSpec],
    index: Option<&crate::core::index::PackageIndex>,
    db: &DbHandle,
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
            .and_then(|s| s.version.clone());

        let target_version = if let Some(index_ref) = index {
            if let Some(entry) = index_ref.find(name) {
                match &requested_version {
                    Some(v) if v.as_str() == "latest" => {
                        let latest = entry.latest().ok_or_else(|| {
                            InstallError::Validation("No releases found for package".to_string())
                        })?;
                        Version::from(latest.version.clone())
                    }
                    Some(v) => v.clone(),
                    None => {
                        let latest = entry.latest().ok_or_else(|| {
                            InstallError::Validation("No releases found for package".to_string())
                        })?;
                        Version::from(latest.version.clone())
                    }
                }
            } else {
                requested_version
                    .clone()
                    .unwrap_or_else(|| Version::from("latest".to_string()))
            }
        } else {
            requested_version
                .clone()
                .unwrap_or_else(|| Version::from("latest".to_string()))
        };

        if let Ok(Some(installed)) = db
            .get_package_version(name.to_string(), target_version.to_string())
            .await
        {
            if installed.active {
                tasks.push(InstallTask::AlreadyInstalled(
                    name.clone(),
                    target_version.clone(),
                ));
                continue;
            } else {
                tasks.push(InstallTask::Switch(name.clone(), target_version.clone()));
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

    let db = DbHandle::spawn().map_err(|e| InstallError::context("Failed to open database", e))?;

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
    let tasks = plan_install_tasks(&resolved_names, &specs, index.as_ref(), &db).await?;

    if tasks.is_empty() {
        return Ok(());
    }

    let table_items: Vec<(PackageName, Option<Version>)> = tasks
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
    let db_clone = db.clone();

    let mut already_installed_count = 0;
    for task in &tasks {
        match task {
            InstallTask::AlreadyInstalled(name, version) => {
                let size = if !dry_run {
                    db_clone
                        .get_package_version(name.to_string(), version.to_string())
                        .await
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

    let to_download: Vec<(PackageName, Option<Version>)> = tasks
        .iter()
        .filter_map(|t| match t {
            InstallTask::Download(n, v) => Some((n.clone(), v.clone())),
            _ => None,
        })
        .collect();

    if !to_download.is_empty() {
        let mut set: tokio::task::JoinSet<Result<Option<PackageName>, InstallError>> =
            tokio::task::JoinSet::new();

        for (name, version) in to_download {
            let client = client.clone();
            let index = index_arc.clone();
            let reporter = reporter.clone();
            let db_task_clone = db.clone();
            let install_count = install_count.clone();

            // Implementation Note: Parallel Downloads
            //
            // We use `JoinSet` here to run downloads concurrently. The concurrency limit
            // is implicitly controlled by the reqwest connection pool (set to 20 earlier).
            // Each task is independent; they don't share state except via the DB (which is locked)
            // and the filesystem (which they write to unique temp dirs).
            set.spawn(async move {
                let unresolved = UnresolvedPackage::new(name, version);
                let resolved = unresolved.resolve(index.as_ref().as_ref())?;

                if dry_run {
                    reporter.done(&resolved.name, &resolved.version, "installed", None);
                    return Ok(None);
                }

                let prepared = resolved.prepare(&client, &reporter).await?;
                let pkg_name = prepared.resolved.name.clone();
                let pkg_version = prepared.resolved.version.clone();

                reporter.installing(&pkg_name, &pkg_version);

                let installer = get_installer(&prepared);
                let info = installer.install(prepared).await?;

                let result = commit_installation(&db_task_clone, &info, &reporter).await;

                if result.is_ok() {
                    reporter.done(&pkg_name, &pkg_version, "installed", Some(info.size_bytes));
                    install_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    Ok(Some(pkg_name))
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

    let all_installed: Vec<PackageName> = tasks
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

// prepare_download removed, replaced by flow::UnresolvedPackage::resolve and ResolvedPackage::prepare

struct InstallInfo {
    package: PackageInfo,
    sha256: String,
    files_to_record: Vec<(String, String)>,
    size_bytes: u64,
}

#[async_trait::async_trait]
trait Installer {
    async fn install(&self, pkg: PreparedPackage) -> Result<InstallInfo, InstallError>;
}

struct BinInstaller;
struct AppInstaller;
struct PkgInstaller;
struct ScriptInstaller;

#[async_trait::async_trait]
impl Installer for BinInstaller {
    async fn install(&self, pkg: PreparedPackage) -> Result<InstallInfo, InstallError> {
        let sha256_copy = pkg.resolved.artifact.hash().to_string();
        let (package_def, pkg_store_path, size_bytes) =
            tokio::task::spawn_blocking(move || install_to_store_only(pkg))
                .await
                .map_err(|e| InstallError::Other(format!("Task panic: {e}")))??;

        relink_macho_files(&pkg_store_path);
        let files_to_record = link_binaries(&package_def.install.bin, &pkg_store_path)?;

        Ok(InstallInfo {
            package: package_def.package,
            sha256: sha256_copy,
            files_to_record,
            size_bytes,
        })
    }
}

#[async_trait::async_trait]
impl Installer for AppInstaller {
    async fn install(&self, pkg: PreparedPackage) -> Result<InstallInfo, InstallError> {
        match tokio::task::spawn_blocking(move || perform_app_install(pkg)).await {
            Ok(res) => res,
            Err(e) => Err(InstallError::Other(format!("Task panic: {e}"))),
        }
    }
}

#[async_trait::async_trait]
impl Installer for PkgInstaller {
    async fn install(&self, pkg: PreparedPackage) -> Result<InstallInfo, InstallError> {
        match tokio::task::spawn_blocking(move || perform_app_install(pkg)).await {
            Ok(res) => res,
            Err(e) => Err(InstallError::Other(format!("Task panic: {e}"))),
        }
    }
}

#[async_trait::async_trait]
impl Installer for ScriptInstaller {
    async fn install(&self, pkg: PreparedPackage) -> Result<InstallInfo, InstallError> {
        let sha256_copy = pkg.resolved.artifact.hash().to_string();
        let (package_def, pkg_store_path, size_bytes) =
            tokio::task::spawn_blocking(move || install_to_store_only(pkg))
                .await
                .map_err(|e| InstallError::Other(format!("Task panic: {e}")))??;

        relink_macho_files(&pkg_store_path);
        let files_to_record = link_binaries(&package_def.install.bin, &pkg_store_path)?;

        Ok(InstallInfo {
            package: package_def.package,
            sha256: sha256_copy,
            files_to_record,
            size_bytes,
        })
    }
}

// Shared link_binaries used from crate::ops

fn get_installer(pkg: &PreparedPackage) -> Box<dyn Installer + Send + Sync> {
    let strategy = pkg.resolved.def.install.strategy.clone();

    let is_app = strategy == InstallStrategy::App
        || pkg
            .extracted_path
            .to_string_lossy()
            .to_lowercase()
            .ends_with(".dmg");

    if is_app {
        Box::new(AppInstaller)
    } else if strategy == InstallStrategy::Pkg {
        Box::new(PkgInstaller)
    } else if pkg.resolved.artifact.is_source() {
        Box::new(ScriptInstaller)
    } else {
        Box::new(BinInstaller)
    }
}

/// Publicly exposed helper for 'apl shell': moves package to store but does NOT link globally.
/// Note: This function does NOT support App or Pkg install strategies.
/// The caller (perform_local_install) routes those to perform_app_install.
pub fn install_to_store_only(
    pkg: PreparedPackage,
) -> Result<(Package, PathBuf, u64), InstallError> {
    let package_def = &pkg.resolved.def;

    let pkg_store_path = store_path()
        .join(&pkg.resolved.name)
        .join(&pkg.resolved.version);
    if pkg_store_path.exists() {
        std::fs::remove_dir_all(&pkg_store_path).map_err(InstallError::Io)?;
    }
    std::fs::create_dir_all(pkg_store_path.parent().unwrap()).map_err(InstallError::Io)?;

    if pkg.resolved.artifact.is_source() {
        perform_source_build(&pkg, &pkg_store_path, package_def)?;
    } else {
        std::fs::rename(&pkg.extracted_path, &pkg_store_path).map_err(|_| {
            InstallError::Other("Cross-volume move failed. APL requires store to be on the same volume as temp dir.".to_string())
        })?;
    }

    if !pkg.resolved.artifact.is_source() {
        let _ = crate::io::extract::strip_components(&pkg_store_path);
    }

    // We intentionally do NOT relink_macho_files here yet?
    // Actually we should, to make them runnable.
    relink_macho_files(&pkg_store_path);

    // Write package metadata for apl shell bin path lookup
    let meta = serde_json::json!({
        "name": pkg.resolved.name,
        "version": pkg.resolved.version,
        "bin": package_def.install.bin,
    });
    let meta_path = pkg_store_path.join(".apl-meta.json");
    if let Ok(meta_content) = serde_json::to_string_pretty(&meta) {
        let _ = std::fs::write(&meta_path, meta_content);
    }

    let size_bytes = calculate_dir_size(&pkg_store_path);
    let def_clone = package_def.clone();

    Ok((def_clone, pkg_store_path, size_bytes))
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

    let log_path = crate::build_log_path(&pkg.resolved.name, &pkg.resolved.version);
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
    let app_name = pkg.resolved.def.install.app.as_ref().ok_or_else(|| {
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

    let extracted_app = if search_path.extension().is_some_and(|e| e == "app") {
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
            if entry.path().extension().is_some_and(|e| e == "app") {
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

    if std::fs::rename(&extracted_app, &target_app).is_err() {
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
        package: pkg.resolved.def.package.clone(),
        sha256: pkg.resolved.artifact.hash().to_string(),
        files_to_record: vec![(
            target_app.to_string_lossy().to_string(),
            "APP_BUNDLE".to_string(),
        )],
        size_bytes: 0,
    })
}

async fn commit_installation(
    db: &DbHandle,
    info: &InstallInfo,
    _reporter: &impl Reporter,
) -> Result<(), InstallError> {
    db.install_complete_package(
        info.package.name.to_string(),
        info.package.version.to_string(),
        info.sha256.clone(),
        info.size_bytes,
        vec![],
        info.files_to_record.clone(),
    )
    .await
    .map_err(|e| InstallError::context("Failed to record installation in DB", e))?;

    db.add_history(
        info.package.name.to_string(),
        "install".to_string(),
        None,
        Some(info.package.version.to_string()),
        true,
    )
    .await
    .map_err(|e| InstallError::context("Failed to add history entry", e))
}

pub fn perform_ux_checks(names: &[PackageName], reporter: &impl Reporter) {
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
            let is_dylib = entry.path().extension().is_some_and(|e| e == "dylib");
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
