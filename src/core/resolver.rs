use crate::core::index::PackageIndex;
use crate::types::PackageName;
use anyhow::{Context, Result, bail};
use std::collections::{HashMap, HashSet, VecDeque};

/// Resolves dependencies for a set of packages and returns them in installation order.
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
pub fn resolve_build_plan(index: &PackageIndex) -> Result<Vec<Vec<PackageName>>> {
    let mut adjacency: HashMap<PackageName, Vec<PackageName>> = HashMap::new();
    let mut in_degree: HashMap<PackageName, usize> = HashMap::new();
    let mut all_packages = HashSet::new();

    for entry in &index.packages {
        let pkg_name = PackageName::new(&entry.name);
        all_packages.insert(pkg_name.clone());

        if let Some(latest) = entry.latest() {
            for dep in &latest.build_deps {
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
    let total_sorted: usize = layers.iter().map(|l| l.len()).sum();
    if total_sorted < all_packages.len() {
        bail!("Circular dependency detected in build graph");
    }

    Ok(layers)
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
    use crate::core::index::{IndexEntry, VersionInfo};

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
            description: "".into(),
            homepage: "https://example.com".into(),
            type_: "cli".into(),
            bins: vec![],
            releases: vec![VersionInfo {
                version: "1.0.0".into(),
                binaries: vec![],
                deps,
                bin: vec![],
                hints: "".into(),
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
