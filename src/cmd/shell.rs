use anyhow::{Context, Result, anyhow};
use apl::core::index::PackageIndex;
use apl::core::manifest::{Lockfile, Manifest};
use apl::ui::Output;
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

pub async fn shell(frozen: bool, update: bool, command: Option<Vec<String>>) -> Result<()> {
    let output = Output::new();

    // 1. Find apl.toml (Manifest)
    let cwd = env::current_dir().context("Failed to get current directory")?;
    let manifest_path = find_manifest(&cwd)
        .ok_or_else(|| anyhow!("apl.toml not found in current or parent directories"))?;
    let root_dir = manifest_path.parent().unwrap();

    output.info(&format!("Found manifest at {}", manifest_path.display()));

    // 2. Load Manifest and Index (once for all branches)
    let manifest = Manifest::load(&manifest_path).await?;
    let index = load_index()?;

    // 3. Resolve Dependencies (Lockfile)
    let lock_path = root_dir.join("apl.lock");
    let existing_lockfile = Lockfile::load(&lock_path).await?;

    // Validate flag combination
    if frozen && update {
        return Err(anyhow!("Cannot use --frozen and --update together"));
    }

    // Determine lockfile to use
    let lockfile = if frozen {
        // Frozen mode: fail if lockfile doesn't exist or is stale
        if existing_lockfile.package.is_empty() && !manifest.dependencies.is_empty() {
            return Err(anyhow!(
                "--frozen: Lockfile is missing or empty. Run 'apl shell' without --frozen first."
            ));
        }
        if !is_lockfile_synced(&manifest, &existing_lockfile) {
            return Err(anyhow!(
                "--frozen: Lockfile is out of sync with manifest. Run 'apl shell' without --frozen to update."
            ));
        }
        output.info("Lockfile is frozen and valid");
        existing_lockfile
    } else if !update && is_lockfile_synced(&manifest, &existing_lockfile) {
        output.info("Lockfile is up to date");
        existing_lockfile
    } else {
        if update {
            output.info("Updating dependencies (--update)...");
        } else {
            output.info("Resolving dependencies...");
        }
        let resolved_lock =
            apl::ops::resolve::resolve_project(&manifest, &index, Some(&existing_lockfile))?;
        resolved_lock
            .save(&lock_path)
            .await
            .context("Failed to save apl.lock")?;
        resolved_lock
    };

    // 4. Ensure Installed (in store)
    let client = reqwest::Client::new();
    ensure_installed(&lockfile, &index, &output, &client).await?;

    run_shell(&output, &lockfile, root_dir, command)
}

/// Load the package index from APL home
fn load_index() -> Result<PackageIndex> {
    let index_path = apl::apl_home().join("index.bin");
    PackageIndex::load(&index_path).context("Failed to load index. Run 'apl update' first.")
}

/// Spawns the shell with the configured environment
fn run_shell(
    output: &Output,
    lockfile: &Lockfile,
    root_dir: &Path,
    command: Option<Vec<String>>,
) -> Result<()> {
    // Construct PATH using package metadata or heuristics
    let mut new_path_entries = Vec::new();
    for pkg in &lockfile.package {
        let store_dir = apl::store_path().join(&pkg.name).join(&pkg.version);

        // Try metadata file first, then heuristic
        let path_to_add = get_bin_dir_from_meta(&store_dir).unwrap_or_else(|| {
            let bin_heuristic = store_dir.join("bin");
            if bin_heuristic.exists() {
                bin_heuristic
            } else {
                store_dir.clone()
            }
        });

        new_path_entries.push(path_to_add);
    }

    let current_path = env::var_os("PATH").unwrap_or_default();
    let mut all_paths = new_path_entries;
    all_paths.extend(env::split_paths(&current_path));

    let new_path = env::join_paths(all_paths).context("Failed to check join paths")?;

    // 6. Spawn Shell
    let shell_bin = env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());

    // Get project name for prompt prefix
    let project_name = root_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "apl".to_string());
    let ps1_prefix = format!("(apl:{project_name}) ");

    output.success("Entering apl shell environment...");

    // Helper to set common env vars
    let set_env = |cmd: &mut Command| {
        cmd.env("PATH", &new_path)
            .env("APL_PROJECT_ROOT", root_dir)
            .env("APL_PS1_PREFIX", &ps1_prefix);
    };

    let status = match command {
        Some(ref args) if !args.is_empty() => {
            // Run specific command
            let (prog, rest) = args.split_first().unwrap();
            let mut cmd = Command::new(prog);
            cmd.args(rest);
            set_env(&mut cmd);
            cmd.status()?
        }
        _ => {
            // Interactive shell
            let mut cmd = Command::new(&shell_bin);
            set_env(&mut cmd);
            cmd.status()?
        }
    };

    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }

    output.info("Exited apl shell.");

    Ok(())
}

fn find_manifest(start: &Path) -> Option<PathBuf> {
    let mut current = start;
    loop {
        let p = current.join("apl.toml");
        if p.exists() {
            return Some(p);
        }
        if let Some(parent) = current.parent() {
            current = parent;
        } else {
            return None;
        }
    }
}

/// Check if lockfile already satisfies all manifest dependencies using semver
fn is_lockfile_synced(manifest: &Manifest, lockfile: &Lockfile) -> bool {
    if lockfile.package.is_empty() && !manifest.dependencies.is_empty() {
        return false;
    }

    for (name, version_req) in &manifest.dependencies {
        let locked = lockfile.package.iter().find(|p| &p.name == name);

        match locked {
            Some(pkg) => {
                if !apl::core::version::version_satisfies_requirement(&pkg.version, version_req) {
                    return false;
                }
            }
            None => return false,
        }
    }

    true
}

/// Extract bin directory from .apl-meta.json if available
fn get_bin_dir_from_meta(store_dir: &Path) -> Option<PathBuf> {
    let meta_path = store_dir.join(".apl-meta.json");
    let content = std::fs::read_to_string(&meta_path).ok()?;
    let meta: serde_json::Value = serde_json::from_str(&content).ok()?;
    let bins = meta.get("bin")?.as_array()?;
    let first_bin = bins.first()?.as_str()?;
    let parent = Path::new(first_bin).parent()?;

    if parent.as_os_str().is_empty() {
        None
    } else {
        Some(store_dir.join(parent))
    }
}

async fn ensure_installed(
    lock: &Lockfile,
    index: &PackageIndex,
    output: &Output,
    client: &reqwest::Client,
) -> Result<()> {
    for pkg in &lock.package {
        let store_dir = apl::store_path().join(&pkg.name).join(&pkg.version);
        if store_dir.exists() {
            continue;
        }

        output.installing(&pkg.name, &pkg.version);

        let unresolved =
            apl::ops::flow::UnresolvedPackage::new(pkg.name.clone(), Some(pkg.version.clone()));
        let resolved = unresolved.resolve(Some(index))?;
        let prepared = resolved.prepare(client, output).await?;

        apl::ops::install::install_to_store_only(prepared)?;

        output.done(&pkg.name, &pkg.version, "ready", None);
    }

    Ok(())
}
