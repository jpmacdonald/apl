use anyhow::{Context, Result};
use apl_core::manifest::{LockPackage, Lockfile, Manifest};
use apl_core::resolver::find_best_match;
use apl_schema::index::PackageIndex;
use apl_schema::{
    Arch,
    types::{PackageName, Version},
};
use std::collections::HashSet;

/// Resolve manifest dependencies against the index to produce a Lockfile
/// This includes transitive dependencies (deps of deps).
/// Optionally accepts existing lockfile to preserve timestamps for unchanged packages.
pub fn resolve_project(
    manifest: &Manifest,
    index: &PackageIndex,
    existing: Option<&Lockfile>,
) -> Result<Lockfile> {
    tracing::debug!("Resolving {} dependencies", manifest.dependencies.len());

    let mut locked_packages = Vec::new();
    let mut visited = HashSet::new();

    // Resolve all direct dependencies
    for (name, version_req) in &manifest.dependencies {
        tracing::debug!("Resolving {} @ {}", name, version_req);
        resolve_package_recursive(
            name,
            version_req,
            index,
            &mut locked_packages,
            &mut visited,
            None,
            existing,
        )?;
    }

    // Sort alphabetically for deterministic output
    locked_packages.sort_by(|a, b| a.name.cmp(&b.name));

    tracing::debug!("Resolved {} packages", locked_packages.len());

    Ok(Lockfile {
        package: locked_packages,
    })
}

/// Recursively resolve a package and its transitive dependencies
fn resolve_package_recursive(
    name: &PackageName,
    version_req: &str,
    index: &PackageIndex,
    locked: &mut Vec<LockPackage>,
    visited: &mut HashSet<PackageName>,
    parent: Option<&PackageName>,
    existing: Option<&Lockfile>, // For timestamp preservation
) -> Result<()> {
    // Skip if already resolved (prevents infinite loops)
    if visited.contains(name) {
        tracing::trace!("Skipping {} (already visited)", name);
        return Ok(());
    }
    visited.insert(name.clone());

    let entry = index.find(name).with_context(|| {
        if let Some(p) = parent {
            format!("Package '{name}' not found in index (required by '{p}')")
        } else {
            format!("Package '{name}' not found in index")
        }
    })?;

    let version_info = find_best_match(entry, version_req)
        .with_context(|| format!("No version found for '{name}' matching '{version_req}'"))?;

    // Find the binary URL/hash for the current platform
    let target_arch = Arch::current();

    let binary = version_info
        .binaries
        .iter()
        .find(|b| b.arch == target_arch)
        .or_else(|| {
            version_info
                .binaries
                .iter()
                .find(|b| b.arch == Arch::Universal)
        })
        .with_context(|| {
            format!(
                "No compatible binary found for package '{}' version '{}' on {}",
                name, version_info.version, target_arch
            )
        })?;

    // Preserve timestamp from existing lockfile if version unchanged
    let timestamp = existing
        .and_then(|lock| {
            lock.package
                .iter()
                .find(|p| p.name == *name && p.version == version_info.version)
        })
        .and_then(|p| p.timestamp)
        .unwrap_or_else(|| chrono::Utc::now().timestamp());

    locked.push(LockPackage {
        name: name.clone(),
        version: Version::from(version_info.version.clone()),
        url: binary.url.clone(),
        sha256: binary.hash.to_string(),
        timestamp: Some(timestamp),
    });

    // Recursively resolve transitive dependencies
    for dep in &version_info.deps {
        // For now, use "latest" for transitive deps (they don't have version reqs in index)
        resolve_package_recursive(
            &PackageName::from(dep.clone()),
            "latest",
            index,
            locked,
            visited,
            Some(name),
            existing,
        )?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use apl_schema::index::{IndexBinary, IndexEntry, VersionInfo};

    fn make_entry(name: &str, versions: Vec<&str>) -> IndexEntry {
        IndexEntry {
            name: name.to_string(),
            description: "test".to_string(),
            homepage: String::new(),
            type_: "cli".to_string(),
            bins: vec![],
            releases: versions
                .into_iter()
                .map(|v| VersionInfo {
                    version: v.to_string(),
                    binaries: vec![IndexBinary {
                        arch: apl_schema::Arch::current(),
                        url: "http://test".to_string(),
                        hash: apl_schema::Sha256Hash::new("hash"),
                        hash_type: apl_schema::index::HashType::Sha256,
                    }],
                    deps: vec![],
                    build_deps: vec![],
                    build_script: String::new(),
                    bin: vec![],
                    hints: String::new(),
                    app: None,
                    source: None,
                })
                .collect(),
            tags: vec![],
        }
    }

    #[test]
    fn test_prefix_match() {
        let entry = make_entry("node", vec!["20.0.0", "20.12.0", "18.0.0"]);
        let best = find_best_match(&entry, "20").unwrap();
        assert_eq!(best.version, "20.12.0");
    }

    #[test]
    fn test_exact_match() {
        let entry = make_entry("node", vec!["20.0.0", "20.12.0", "18.0.0"]);
        let best = find_best_match(&entry, "18.0.0").unwrap();
        assert_eq!(best.version, "18.0.0");
    }

    #[test]
    fn test_latest_match() {
        let mut entry = make_entry("node", vec!["18.0.0", "20.12.0"]);
        // releases are sorted by upsert usually, but here we manually constructed.
        // find_best_match depends on 'is_newer' sort for prefix, but 'latest' relies on index order
        // so let's ensure order in test construction matches index expectation (descending)
        entry.releases.sort_by(|a, b| b.version.cmp(&a.version));

        let best = find_best_match(&entry, "latest").unwrap();
        assert_eq!(best.version, "20.12.0");
    }

    #[test]
    fn test_segment_match_does_not_match_longer_prefix() {
        // "0.2" should NOT match "0.20.0"
        let entry = make_entry("bat", vec!["0.2.0", "0.20.0", "0.2.1"]);
        let best = find_best_match(&entry, "0.2").unwrap();
        // Should get 0.2.1 (newest in 0.2.x), NOT 0.20.0
        assert_eq!(best.version, "0.2.1");
    }

    #[test]
    fn test_segment_match_single_segment() {
        // "1" should match "1.0.0" and "1.2.3" but NOT "10.0.0"
        let entry = make_entry("foo", vec!["1.0.0", "1.2.3", "10.0.0", "2.0.0"]);
        let best = find_best_match(&entry, "1").unwrap();
        assert_eq!(best.version, "1.2.3");
    }

    #[test]
    fn test_timestamp_preservation() {
        use apl_core::manifest::LockPackage;

        // Create a mock existing lockfile with a timestamp
        let old_timestamp = 1_700_000_000_i64;
        let existing = Lockfile {
            package: vec![LockPackage {
                name: PackageName::from("node".to_string()),
                version: Version::from("20.12.0".to_string()),
                url: "http://old".to_string(),
                sha256: "oldhash".to_string(),
                timestamp: Some(old_timestamp),
            }],
        };

        // Create index with same version
        let mut index = PackageIndex::default();
        let entry = make_entry("node", vec!["20.12.0"]);
        index.packages.push(entry);

        // Create manifest requesting same version
        let manifest = Manifest {
            project: apl_core::manifest::ProjectObj {
                name: "test".to_string(),
            },
            dependencies: [(PackageName::from("node".to_string()), "20".to_string())]
                .into_iter()
                .collect(),
        };

        // Resolve with existing lockfile
        let result = resolve_project(&manifest, &index, Some(&existing)).unwrap();

        // Timestamp should be preserved since version is unchanged
        assert_eq!(result.package[0].timestamp, Some(old_timestamp));
    }
}
