use crate::core::index::{IndexEntry, PackageIndex, VersionInfo};
use crate::core::manifest::{LockPackage, Lockfile, Manifest};
use crate::core::version::is_newer;
use crate::{Arch, PackageName, Version};
use anyhow::{Context, Result};
use std::collections::HashSet;
use std::str::FromStr;

/// Universal binary architecture for macOS
const UNIVERSAL_ARCH: &str = "universal-apple-darwin";

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
    let target_arch = crate::Arch::current();

    let binary = version_info
        .binaries
        .iter()
        .find(|b| Arch::from_str(&b.arch).ok() == Some(target_arch))
        .or_else(|| {
            version_info
                .binaries
                .iter()
                .find(|b| b.arch == UNIVERSAL_ARCH)
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
        blake3: binary.hash.clone(),
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

/// Find the best matching version for a requirement
/// Supports:
/// - "latest" or "*": newest version
/// - "1.2.3": exact match
/// - "^1.2": compatible (>=1.2.0, <2.0.0)
/// - "~1.2": minor-compatible (>=1.2.0, <1.3.0)
/// - ">=1.0, <2.0": range
/// - "1.2": prefix match (backward compat)
fn find_best_match<'a>(entry: &'a IndexEntry, requirement: &str) -> Option<&'a VersionInfo> {
    if requirement == "latest" || requirement == "*" {
        return entry.latest();
    }

    // Exact match first
    if let Some(v) = entry.find_version(requirement) {
        return Some(v);
    }

    // Try parsing as semver requirement (^, ~, ranges)
    if let Ok(version_req) = semver::VersionReq::parse(requirement) {
        let mut candidates: Vec<&VersionInfo> = entry
            .releases
            .iter()
            .filter(|r| {
                semver::Version::parse(&r.version)
                    .map(|v| version_req.matches(&v))
                    .unwrap_or(false)
            })
            .collect();

        if !candidates.is_empty() {
            // Sort descending: newer versions first
            candidates.sort_by(|a, b| {
                match (
                    semver::Version::parse(&a.version),
                    semver::Version::parse(&b.version),
                ) {
                    (Ok(va), Ok(vb)) => vb.cmp(&va),
                    _ => std::cmp::Ordering::Equal,
                }
            });
            return candidates.first().copied();
        }
    }

    // Fallback: segment-based prefix match for non-semver specs (e.g. "20" for node)
    let mut candidates: Vec<&VersionInfo> = entry
        .releases
        .iter()
        .filter(|r| crate::core::version::version_matches_segments(&r.version, requirement))
        .collect();

    if candidates.is_empty() {
        return None;
    }

    // Sort descending: newer versions first
    candidates.sort_by(|a, b| {
        if is_newer(&a.version, &b.version) {
            std::cmp::Ordering::Greater
        } else if is_newer(&b.version, &a.version) {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Equal
        }
    });

    candidates.first().copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::index::{IndexBinary, VersionInfo};

    fn make_entry(name: &str, versions: Vec<&str>) -> IndexEntry {
        IndexEntry {
            name: name.to_string(),
            description: "test".to_string(),
            homepage: "".to_string(),
            type_: "cli".to_string(),
            releases: versions
                .into_iter()
                .map(|v| VersionInfo {
                    version: v.to_string(),
                    binaries: vec![IndexBinary {
                        arch: crate::Arch::current().as_str().to_string(),
                        url: "http://test".to_string(),
                        hash: "hash".to_string(),
                        hash_type: crate::core::index::HashType::Blake3,
                    }],
                    deps: vec![],
                    build_deps: vec![],
                    build_script: "".to_string(),
                    bin: vec![],
                    hints: "".to_string(),
                    app: None,
                    source: None,
                })
                .collect(),
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
        use crate::core::manifest::LockPackage;

        // Create a mock existing lockfile with a timestamp
        let old_timestamp = 1700000000i64;
        let existing = Lockfile {
            package: vec![LockPackage {
                name: PackageName::from("node".to_string()),
                version: Version::from("20.12.0".to_string()),
                url: "http://old".to_string(),
                blake3: "oldhash".to_string(),
                timestamp: Some(old_timestamp),
            }],
        };

        // Create index with same version
        let mut index = PackageIndex::default();
        let entry = make_entry("node", vec!["20.12.0"]);
        index.packages.push(entry);

        // Create manifest requesting same version
        let manifest = Manifest {
            project: crate::core::manifest::ProjectObj {
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
