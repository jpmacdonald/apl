use crate::core::index::PackageIndex;
use crate::types::PackageName;
use anyhow::{Context, Result, bail};
use std::collections::HashSet;

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
}
