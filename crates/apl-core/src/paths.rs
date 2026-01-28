use dirs::home_dir;
use std::path::PathBuf;

/// Returns the primary configuration directory, or None if the user's home cannot be resolved.
pub fn try_apl_home() -> Option<PathBuf> {
    if let Ok(val) = std::env::var("APL_HOME") {
        return Some(PathBuf::from(val));
    }
    home_dir().map(|h| h.join(".apl"))
}

/// Returns the canonical APL home directory (`~/.apl`).
///
/// # Panics
///
/// Panics if neither `APL_HOME` is set nor the user's home directory can be
/// resolved. On macOS this should never happen in normal use.
pub fn apl_home() -> PathBuf {
    try_apl_home().expect("Could not determine home directory. Set APL_HOME to override.")
}

/// `SQLite` database path: ~/.apl/state.db
pub fn db_path() -> PathBuf {
    apl_home().join("state.db")
}

/// Package store path: ~/.apl/store
pub fn store_path() -> PathBuf {
    apl_home().join("store")
}

/// Binary installation target: ~/.apl/bin
pub fn bin_path() -> PathBuf {
    apl_home().join("bin")
}

/// Cache path: ~/.apl/cache
pub fn cache_path() -> PathBuf {
    apl_home().join("cache")
}

/// Logs directory: ~/.apl/logs
pub fn log_dir() -> PathBuf {
    apl_home().join("logs")
}

/// Registry templates directory: ~/.apl/registry
pub fn registry_dir() -> PathBuf {
    apl_home().join("registry")
}

/// Generate a build log path for a package
pub fn build_log_path(package: &str, version: &str) -> PathBuf {
    let timestamp = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    log_dir().join(format!("build-{package}-{version}-{timestamp}.log"))
}

/// Temp path: ~/.apl/tmp (guaranteed same volume as store)
pub fn tmp_path() -> PathBuf {
    apl_home().join("tmp")
}

/// Extract the filename from a URL.
pub fn filename_from_url(url: &str) -> &str {
    url.split('/').next_back().unwrap_or("")
}
