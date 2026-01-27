use anyhow::{Context, Result, bail};
use apl_schema::PackageIndex;
use apl_schema::index::{IndexEntry, VersionInfo};
use apl_schema::types::PackageName;
use apl_schema::version::is_newer;
use std::collections::{HashMap, HashSet, VecDeque};

/// Resolves dependencies for a set of packages and returns them in installation order.
///
/// Performs a recursive depth-first traversal of the dependency graph,
/// detecting cycles and producing a topologically sorted list.
///
/// # Errors
///
/// Returns an error if a package is not found in the index or a circular
/// dependency is detected.
pub fn resolve_dependencies(
    pkg_names: &[PackageName],
    index: &PackageIndex,
) -> Result<Vec<PackageName>> {
    let mut resolved_order = Vec::new();
    let mut visited = HashSet::new();
    let mut visiting = HashSet::new();

    for name in pkg_names {
        resolve_recursive(
            name,
            index,
            &mut resolved_order,
            &mut visited,
            &mut visiting,
        )?;
    }

    Ok(resolved_order)
}

/// Resolves a build plan for the entire index, returning layers of packages
/// that can be built in parallel.
///
/// Uses Kahn's algorithm for topological sorting. Each layer contains
/// packages whose dependencies have all been resolved in prior layers.
///
/// # Errors
///
/// Returns an error if a circular dependency is detected in the build graph.
/// # Panics
/// Panics if the internal graph structure is inconsistent (e.g. missing node degree).
pub fn resolve_build_plan(index: &PackageIndex) -> Result<Vec<Vec<PackageName>>> {
    let mut adjacency: HashMap<PackageName, Vec<PackageName>> = HashMap::new();
    let mut in_degree: HashMap<PackageName, usize> = HashMap::new();
    let mut all_packages = HashSet::new();

    for entry in &index.packages {
        let pkg_name = PackageName::new(&entry.name);
        all_packages.insert(pkg_name.clone());

        if let Some(latest) = entry.latest() {
            for dep in latest.build_deps.iter().chain(latest.deps.iter()) {
                let dep_name = PackageName::new(dep);
                adjacency
                    .entry(dep_name.clone())
                    .or_default()
                    .push(pkg_name.clone());
                *in_degree.entry(pkg_name.clone()).or_default() += 1;
                all_packages.insert(dep_name);
            }
        }
    }

    let mut layers = Vec::new();
    let queue: VecDeque<PackageName> = all_packages
        .iter()
        .filter(|p| in_degree.get(*p).copied().unwrap_or(0) == 0)
        .cloned()
        .collect();

    // Sort queue for deterministic output
    let mut sorted_queue: Vec<PackageName> = queue.into_iter().collect();
    sorted_queue.sort();
    let mut queue = VecDeque::from(sorted_queue);

    while !queue.is_empty() {
        let mut current_layer = Vec::new();
        let mut next_queue_vec = Vec::new();

        while let Some(u) = queue.pop_front() {
            current_layer.push(u.clone());

            if let Some(neighbors) = adjacency.get(&u) {
                for v in neighbors {
                    let degree = in_degree.get_mut(v).unwrap();
                    *degree -= 1;
                    if *degree == 0 {
                        next_queue_vec.push(v.clone());
                    }
                }
            }
        }

        if !current_layer.is_empty() {
            current_layer.sort();
            layers.push(current_layer);
        }

        next_queue_vec.sort();
        queue = VecDeque::from(next_queue_vec);
    }

    // Check for cycles
    let total_sorted: usize = layers.iter().map(Vec::len).sum();
    if total_sorted < all_packages.len() {
        bail!("Circular dependency detected in build graph");
    }

    Ok(layers)
}

/// Resolves a package spec (e.g. `pkg@v1.0.0`) against the index.
///
/// If no `@version` suffix is present, the latest version is selected.
///
/// # Errors
///
/// Returns an error if the package is not found in the index or no version
/// matches the requested requirement.
pub fn resolve_package_spec<'a>(
    spec: &str,
    index: &'a PackageIndex,
) -> Result<(&'a IndexEntry, &'a VersionInfo)> {
    let (name_str, version_req) = if let Some(idx) = spec.find('@') {
        (&spec[..idx], &spec[idx + 1..])
    } else {
        (spec, "latest")
    };

    let name = PackageName::new(name_str);
    let entry = index
        .find(&name)
        .with_context(|| format!("Package '{name_str}' not found"))?;

    let version_info = find_best_match(entry, version_req)
        .with_context(|| format!("No version found for '{name_str}' matching '{version_req}'"))?;

    Ok((entry, version_info))
}

/// Find the best matching version for a requirement
/// Supports:
/// - "latest" or "*": newest version
/// - "1.2.3": exact match
/// - "^1.2": compatible (>=1.2.0, <2.0.0)
/// - "~1.2": minor-compatible (>=1.2.0, <1.3.0)
/// - ">=1.0, <2.0": range
/// - "1.2": prefix match (backward compat)
pub fn find_best_match<'a>(entry: &'a IndexEntry, requirement: &str) -> Option<&'a VersionInfo> {
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
                if is_newer(&a.version, &b.version) {
                    std::cmp::Ordering::Greater
                } else if is_newer(&b.version, &a.version) {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Equal
                }
            });
            return Some(candidates[0]);
        }
    }

    // Fallback: prefix match (e.g. "1.2" matches "1.2.3")
    let mut prefix_candidates: Vec<&VersionInfo> = entry
        .releases
        .iter()
        .filter(|r| apl_schema::version::version_matches_segments(&r.version, requirement))
        .collect();

    if prefix_candidates.is_empty() {
        return None;
    }

    // Sort descending: newer versions first
    prefix_candidates.sort_by(|a, b| {
        if is_newer(&a.version, &b.version) {
            std::cmp::Ordering::Greater
        } else if is_newer(&b.version, &a.version) {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Equal
        }
    });

    Some(prefix_candidates[0])
}

fn resolve_recursive(
    name: &PackageName,
    index: &PackageIndex,
    order: &mut Vec<PackageName>,
    visited: &mut HashSet<PackageName>,
    visiting: &mut HashSet<PackageName>,
) -> Result<()> {
    if visited.contains(name) {
        return Ok(());
    }

    if visiting.contains(name) {
        bail!("Circular dependency detected involving package: {name}");
    }

    visiting.insert(name.clone());

    let entry = index
        .find(name)
        .with_context(|| format!("Package '{name}' not found in index"))?;

    let latest = entry
        .latest()
        .with_context(|| format!("Package '{name}' has no releases"))?;
    for dep in &latest.deps {
        let dep_name = PackageName::new(dep);
        resolve_recursive(&dep_name, index, order, visited, visiting)?;
    }

    visiting.remove(name);
    visited.insert(name.clone());
    order.push(name.clone());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use apl_schema::index::{IndexEntry, VersionInfo};

    fn mock_index(entries: Vec<IndexEntry>) -> PackageIndex {
        let mut index = PackageIndex::new();
        for entry in entries {
            index.upsert(entry);
        }
        index
    }

    fn simple_entry(name: &str, deps: Vec<String>) -> IndexEntry {
        IndexEntry {
            name: name.into(),
            description: String::new(),
            homepage: "https://example.com".into(),
            type_: "cli".into(),
            bins: vec![],
            releases: vec![VersionInfo {
                version: "1.0.0".into(),
                binaries: vec![],
                deps,
                bin: vec![],
                hints: String::new(),
                app: None,
                source: None,
                build_deps: vec![],
                build_script: String::new(),
            }],
            tags: vec![],
        }
    }

    #[test]
    fn test_simple_resolution() {
        let index = mock_index(vec![
            simple_entry("a", vec!["b".into()]),
            simple_entry("b", vec![]),
        ]);

        let resolved = resolve_dependencies(&["a".into()], &index).unwrap();
        assert_eq!(resolved, vec!["b", "a"]);
    }

    #[test]
    fn test_complex_resolution() {
        let index = mock_index(vec![
            simple_entry("a", vec!["b".into(), "c".into()]),
            simple_entry("b", vec!["d".into()]),
            simple_entry("c", vec!["d".into()]),
            simple_entry("d", vec![]),
        ]);

        let resolved = resolve_dependencies(&["a".into()], &index).unwrap();
        // d must come before b and c. b and c must come before a.
        assert!(
            resolved.iter().position(|x| x == "d").unwrap()
                < resolved.iter().position(|x| x == "b").unwrap()
        );
        assert!(
            resolved.iter().position(|x| x == "d").unwrap()
                < resolved.iter().position(|x| x == "c").unwrap()
        );
        assert!(
            resolved.iter().position(|x| x == "b").unwrap()
                < resolved.iter().position(|x| x == "a").unwrap()
        );
        assert!(
            resolved.iter().position(|x| x == "c").unwrap()
                < resolved.iter().position(|x| x == "a").unwrap()
        );
    }

    #[test]
    fn test_cycle_detection() {
        let index = mock_index(vec![
            simple_entry("a", vec!["b".into()]),
            simple_entry("b", vec!["a".into()]),
        ]);

        let result = resolve_dependencies(&["a".into()], &index);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Circular dependency")
        );
    }

    #[test]
    fn test_build_plan_layers() {
        let mut entry_a = simple_entry("a", vec![]);
        entry_a.releases[0].build_deps = vec!["b".to_string(), "c".to_string()];

        let mut entry_b = simple_entry("b", vec![]);
        entry_b.releases[0].build_deps = vec!["d".to_string()];

        let entry_c = simple_entry("c", vec![]);
        let entry_d = simple_entry("d", vec![]);

        let index = mock_index(vec![entry_a, entry_b, entry_c, entry_d]);

        let layers = resolve_build_plan(&index).unwrap();
        // Layer 0: c, d (no build deps or deps satisfied)
        // Layer 1: b (depends on d)
        // Layer 2: a (depends on b, c)
        assert_eq!(layers.len(), 3);
        assert_eq!(
            layers[0],
            vec![PackageName::new("c"), PackageName::new("d")]
        );
        assert_eq!(layers[1], vec![PackageName::new("b")]);
        assert_eq!(layers[2], vec![PackageName::new("a")]);
    }

    #[test]
    fn test_deep_build_chain() {
        // a -> b -> c -> d -> e
        let mut entries = Vec::new();
        let names = ["a", "b", "c", "d", "e"];
        for i in 0..names.len() {
            let mut entry = simple_entry(names[i], vec![]);
            if i + 1 < names.len() {
                entry.releases[0].build_deps = vec![names[i + 1].to_string()];
            }
            entries.push(entry);
        }

        let index = mock_index(entries);
        let layers = resolve_build_plan(&index).unwrap();

        assert_eq!(layers.len(), 5);
        assert_eq!(layers[0], vec![PackageName::new("e")]);
        assert_eq!(layers[1], vec![PackageName::new("d")]);
        assert_eq!(layers[2], vec![PackageName::new("c")]);
        assert_eq!(layers[3], vec![PackageName::new("b")]);
        assert_eq!(layers[4], vec![PackageName::new("a")]);
    }
}
