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

use crate::DbHandle;
use crate::ui::Reporter;
use crate::{bin_path, ops::Context, ops::InstallError, ops::link_binaries, store_path};
use apl_core::io::dmg;
use apl_core::package::{InstallStrategy, Package, PackageInfo};
use apl_core::relinker::Relinker;
use apl_schema::types::{PackageName, Version};
use apl_schema::version::PackageSpec;

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

async fn resolve_and_filter_packages(
    packages: &[String],
    ctx: &Context,
) -> Result<(Vec<PackageName>, Vec<PackageSpec>), InstallError> {
    let specs: Vec<PackageSpec> = packages
        .iter()
        .map(|p| PackageSpec::parse(p))
        .collect::<anyhow::Result<Vec<_>>>()
        .map_err(|e| InstallError::Validation(e.to_string()))?;

    let mut valid_names: Vec<PackageName> = Vec::new();
    if let Some(index_ref) = ctx.index.as_deref() {
        for spec in &specs {
            if Path::new(&*spec.name).exists() || index_ref.find(&spec.name).is_some() {
                valid_names.push(spec.name.clone());
            } else {
                ctx.reporter
                    .failed(&spec.name, &Version::from(""), "Package not found in index");
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
        let index_ref = ctx.index.as_ref().ok_or_else(|| {
            InstallError::Validation("No index found. Run 'apl update' first.".to_string())
        })?;

        let mut resolved = apl_core::resolver::resolve_dependencies(&index_names, index_ref)
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
    ctx: &Context,
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

        let target_version = if let Some(index_ref) = ctx.index.as_deref() {
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

        if let Ok(Some(installed)) = ctx
            .db
            .get_package_version(name.to_string(), target_version.to_string())
            .await
        {
            if installed.active {
                tasks.push(InstallTask::AlreadyInstalled(
                    name.clone(),
                    target_version.clone(),
                ));
                continue;
            }
            tasks.push(InstallTask::Switch(name.clone(), target_version.clone()));
            continue;
        }

        tasks.push(InstallTask::Download(name.clone(), Some(target_version)));
    }

    Ok(tasks)
}

/// Resolves, downloads, and installs a set of packages.
pub async fn install_packages(
    ctx: &Context,
    packages: &[String],
    dry_run: bool,
) -> Result<(), InstallError> {
    // Phase 1: Logic - Resolve Dependencies
    let (resolved_names, specs) = resolve_and_filter_packages(packages, ctx).await?;

    // Phase 2: Planning - Determine Actions
    let tasks = plan_install_tasks(&resolved_names, &specs, ctx).await?;

    if tasks.is_empty() {
        return Ok(());
    }

    let table_items: Vec<(PackageName, Option<Version>, usize)> = tasks
        .iter()
        .map(|t| match t {
            InstallTask::Download(n, v) => (n.clone(), v.clone(), 0),
            InstallTask::AlreadyInstalled(n, v) | InstallTask::Switch(n, v) => {
                (n.clone(), Some(v.clone()), 0)
            }
        })
        .collect();

    ctx.reporter.prepare_pipeline(&table_items);

    let start_time = Instant::now();
    let install_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let ctx_clone = ctx.clone();

    let mut already_installed_count = 0;
    for task in &tasks {
        match task {
            InstallTask::AlreadyInstalled(name, version) => {
                let size = if dry_run {
                    None
                } else {
                    ctx.db
                        .get_package_version(name.to_string(), version.to_string())
                        .await
                        .ok()
                        .flatten()
                        .map(|p| p.size_bytes)
                };
                ctx.reporter.done(name, version, "installed", size);
                already_installed_count += 1;
            }
            InstallTask::Switch(name, version) => {
                ctx.reporter.installing(name, version, None, None);
                if dry_run {
                    ctx.reporter.done(name, version, "(dry run)", None);
                } else {
                    crate::ops::switch::switch_version(name, version, dry_run, &ctx.reporter)
                        .map_err(|e| InstallError::Other(e.to_string()))?;
                    install_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            }
            InstallTask::Download(..) => {}
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
            let ctx = ctx_clone.clone();
            let install_count = install_count.clone();

            // Implementation Note: Parallel Downloads
            //
            // We use `JoinSet` here to run downloads concurrently. The concurrency limit
            // is implicitly controlled by the reqwest connection pool (set to 20 earlier).
            // Each task is independent; they don't share state except via the DB (which is locked)
            // and the filesystem (which they write to unique temp dirs).
            set.spawn(async move {
                let unresolved = UnresolvedPackage::new(name, version);
                let resolved = unresolved.resolve(ctx.index.as_deref())?;

                if dry_run {
                    ctx.reporter
                        .done(&resolved.name, &resolved.version, "installed", None);
                    return Ok(None);
                }

                let prepared = resolved.prepare(&ctx.client, &ctx.reporter).await?;
                let pkg_name = prepared.resolved.name.clone();
                let pkg_version = prepared.resolved.version.clone();

                ctx.reporter.installing(&pkg_name, &pkg_version, None, None);

                let installer = get_installer(&prepared);
                let reporter_arc: Arc<dyn Reporter> = Arc::new(ctx.reporter.clone());
                let info = installer.install(prepared, reporter_arc).await?;

                let result = commit_installation(&ctx.db, &info).await;

                if result.is_ok() {
                    ctx.reporter
                        .done(&pkg_name, &pkg_version, "installed", Some(info.size_bytes));
                    install_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    Ok(Some(pkg_name))
                } else {
                    Ok(None)
                }
            });
        }

        while let Some(res) = set.join_next().await {
            match res {
                Ok(Ok(None | Some(_))) => {}
                Ok(Err(e)) => ctx.reporter.error(&format!("Install failed: {e}")),
                Err(e) => ctx.reporter.error(&format!("Internal error: {e}")),
            }
        }
    }

    let count = install_count.load(std::sync::atomic::Ordering::Relaxed);
    if count > 0 {
        ctx.reporter
            .summary(count, "install", start_time.elapsed().as_secs_f64());
    } else if already_installed_count > 0 {
        ctx.reporter
            .summary_plain(already_installed_count, "already installed");
    }

    let all_installed: Vec<PackageName> = tasks
        .iter()
        .map(|t| match t {
            InstallTask::Download(n, _)
            | InstallTask::Switch(n, _)
            | InstallTask::AlreadyInstalled(n, _) => n.clone(),
        })
        .collect();

    perform_ux_checks(&all_installed, &ctx.reporter);

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
    async fn install(
        &self,
        pkg: PreparedPackage,
        reporter: Arc<dyn Reporter>,
    ) -> Result<InstallInfo, InstallError>;
}

struct BinInstaller;
struct AppInstaller;
struct PkgInstaller;
struct ScriptInstaller;

#[async_trait::async_trait]
impl Installer for BinInstaller {
    async fn install(
        &self,
        pkg: PreparedPackage,
        reporter: Arc<dyn Reporter>,
    ) -> Result<InstallInfo, InstallError> {
        let sha256_copy = pkg.resolved.artifact.hash().to_string();
        let (package_def, pkg_store_path, size_bytes) =
            tokio::task::spawn_blocking(move || install_to_store_only(pkg, reporter))
                .await
                .map_err(|e| InstallError::Other(format!("Task panic: {e}")))??;

        let bins = package_def.install.effective_bin(&package_def.package.name);
        let files_to_record = link_binaries(&bins, &pkg_store_path)?;

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
    async fn install(
        &self,
        pkg: PreparedPackage,
        _reporter: Arc<dyn Reporter>,
    ) -> Result<InstallInfo, InstallError> {
        match tokio::task::spawn_blocking(move || perform_app_install(pkg)).await {
            Ok(res) => res,
            Err(e) => Err(InstallError::Other(format!("Task panic: {e}"))),
        }
    }
}

#[async_trait::async_trait]
impl Installer for PkgInstaller {
    async fn install(
        &self,
        pkg: PreparedPackage,
        _reporter: Arc<dyn Reporter>,
    ) -> Result<InstallInfo, InstallError> {
        match tokio::task::spawn_blocking(move || perform_app_install(pkg)).await {
            Ok(res) => res,
            Err(e) => Err(InstallError::Other(format!("Task panic: {e}"))),
        }
    }
}

#[async_trait::async_trait]
impl Installer for ScriptInstaller {
    async fn install(
        &self,
        pkg: PreparedPackage,
        reporter: Arc<dyn Reporter>,
    ) -> Result<InstallInfo, InstallError> {
        let sha256_copy = pkg.resolved.artifact.hash().to_string();
        let (package_def, pkg_store_path, size_bytes) =
            tokio::task::spawn_blocking(move || install_to_store_only(pkg, reporter))
                .await
                .map_err(|e| InstallError::Other(format!("Task panic: {e}")))??;

        let bins = package_def.install.effective_bin(&package_def.package.name);
        let files_to_record = link_binaries(&bins, &pkg_store_path)?;

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
    let strategy = pkg
        .resolved
        .def
        .install
        .strategy
        .clone()
        .unwrap_or(InstallStrategy::Link);

    let is_app = strategy == InstallStrategy::App
        || pkg.resolved.def.install.app.is_some()
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
// pkg and reporter are taken by value intentionally: pkg owns a TempDir that
// must stay alive for the duration of the install, and reporter is an Arc
// shared across threads.
#[allow(clippy::needless_pass_by_value)]
pub fn install_to_store_only(
    pkg: PreparedPackage,
    reporter: Arc<dyn Reporter>,
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
        let _ = apl_core::io::extract::strip_components(&pkg_store_path);
    }

    // We intentionally do NOT relink_macho_files here yet?
    // Actually we should, to make them runnable.
    relink_macho_files(
        &pkg_store_path,
        &pkg.resolved.name,
        &pkg.resolved.version,
        &reporter,
    );

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
        apl_core::sysroot::Sysroot::new().map_err(|e| InstallError::Other(e.to_string()))?;
    let builder = apl_core::builder::Builder::new(&sysroot);
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
            &[], // No build-time dependency mounting on client yet
            &build_spec.script,
            store_path,
            false,
            &log_path,
            None, // Host architecture
        )
        .map_err(|e| InstallError::Script(e.to_string()))
}

// pkg is taken by value to keep the TempDir alive during installation.
#[allow(clippy::needless_pass_by_value)]
fn perform_app_install(pkg: PreparedPackage) -> Result<InstallInfo, InstallError> {
    let app_name = pkg.resolved.def.install.app.as_ref().ok_or_else(|| {
        InstallError::Validation("type='app' requires [install] app='Name.app'".to_string())
    })?;

    let applications_dir = dirs::home_dir().map_or_else(
        || PathBuf::from("/Applications"),
        |h| h.join("Applications"),
    );
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
        apl_core::builder::copy_dir_all(&extracted_app, &target_app)
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

async fn commit_installation(db: &DbHandle, info: &InstallInfo) -> Result<(), InstallError> {
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
        .filter(std::fs::Metadata::is_file)
        .map(|m| m.len())
        .sum()
}

fn relink_macho_files(
    path: &Path,
    pkg_name: &PackageName,
    pkg_version: &Version,
    reporter: &Arc<dyn Reporter>,
) {
    #[cfg(target_os = "macos")]
    {
        // 1. Collect and count files for progress
        let mut targets = Vec::new();
        for entry in walkdir::WalkDir::new(path).into_iter().flatten() {
            if entry.path().is_file() {
                targets.push(entry.path().to_path_buf());
            }
        }

        let total = targets.len() as u64;
        let mut current = 0;

        // 2. Iterate and process
        for path in targets {
            let is_dylib = path.extension().is_some_and(|e| e == "dylib");
            let is_exec = std::fs::metadata(&path)
                .map(|m| {
                    use std::os::unix::fs::PermissionsExt;
                    m.permissions().mode() & 0o111 != 0
                })
                .unwrap_or(false);

            if is_dylib {
                let _ = Relinker::fix_dylib(&path);
            } else if is_exec {
                let _ = Relinker::fix_binary(&path);
            }

            current += 1;
            // Report progress every 10 files or on completion to avoid flooding channel
            // (Actually the Actor channel is fast, we can just report every time for smooth animation)
            reporter.installing(pkg_name, pkg_version, Some(current), Some(total));
        }
    }
}

fn check_build_deps(deps: &[String]) -> Vec<String> {
    deps.iter()
        .filter(|d| which::which(d).is_err())
        .cloned()
        .collect()
}
