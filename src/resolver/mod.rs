use anyhow::{Result, bail, Context};
use crate::index::PackageIndex;
use std::collections::HashSet;

/// Resolves dependencies for a set of packages and returns them in installation order.
pub fn resolve_dependencies(
    pkg_names: &[String],
    index: &PackageIndex,
) -> Result<Vec<String>> {
    let mut resolved_order = Vec::new();
    let mut visited = HashSet::new();
    let mut visiting = HashSet::new();

    for name in pkg_names {
        resolve_recursive(name, index, &mut resolved_order, &mut visited, &mut visiting)?;
    }

    Ok(resolved_order)
}

fn resolve_recursive(
    name: &str,
    index: &PackageIndex,
    order: &mut Vec<String>,
    visited: &mut HashSet<String>,
    visiting: &mut HashSet<String>,
) -> Result<()> {
    if visited.contains(name) {
        return Ok(());
    }

    if visiting.contains(name) {
        bail!("Circular dependency detected involving package: {}", name);
    }

    visiting.insert(name.to_string());

    let entry = index.find(name)
        .with_context(|| format!("Package '{}' not found in index", name))?;

    for dep in &entry.deps {
        resolve_recursive(dep, index, order, visited, visiting)?;
    }

    visiting.remove(name);
    visited.insert(name.to_string());
    order.push(name.to_string());

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{IndexEntry, IndexBottle};

    fn mock_index(entries: Vec<IndexEntry>) -> PackageIndex {
        let mut index = PackageIndex::new();
        for entry in entries {
            index.upsert(entry);
        }
        index
    }

    #[test]
    fn test_simple_resolution() {
        let index = mock_index(vec![
            IndexEntry {
                name: "a".into(),
                version: "1.0".into(),
                description: "".into(),
                bottles: vec![],
                deps: vec!["b".into()],
                bin: vec![],
            },
            IndexEntry {
                name: "b".into(),
                version: "1.0".into(),
                description: "".into(),
                bottles: vec![],
                deps: vec![],
                bin: vec![],
            },
        ]);

        let resolved = resolve_dependencies(&["a".into()], &index).unwrap();
        assert_eq!(resolved, vec!["b", "a"]);
    }

    #[test]
    fn test_complex_resolution() {
        let index = mock_index(vec![
            IndexEntry { name: "a".into(), version: "1.0".into(), description: "".into(), bottles: vec![], deps: vec!["b".into(), "c".into()], bin: vec![] },
            IndexEntry { name: "b".into(), version: "1.0".into(), description: "".into(), bottles: vec![], deps: vec!["d".into()], bin: vec![] },
            IndexEntry { name: "c".into(), version: "1.0".into(), description: "".into(), bottles: vec![], deps: vec!["d".into()], bin: vec![] },
            IndexEntry { name: "d".into(), version: "1.0".into(), description: "".into(), bottles: vec![], deps: vec![], bin: vec![] },
        ]);

        let resolved = resolve_dependencies(&["a".into()], &index).unwrap();
        // d must come before b and c. b and c must come before a.
        assert!(resolved.iter().position(|x| x == "d").unwrap() < resolved.iter().position(|x| x == "b").unwrap());
        assert!(resolved.iter().position(|x| x == "d").unwrap() < resolved.iter().position(|x| x == "c").unwrap());
        assert!(resolved.iter().position(|x| x == "b").unwrap() < resolved.iter().position(|x| x == "a").unwrap());
        assert!(resolved.iter().position(|x| x == "c").unwrap() < resolved.iter().position(|x| x == "a").unwrap());
    }

    #[test]
    fn test_cycle_detection() {
        let index = mock_index(vec![
            IndexEntry { name: "a".into(), version: "1.0".into(), description: "".into(), bottles: vec![], deps: vec!["b".into()], bin: vec![] },
            IndexEntry { name: "b".into(), version: "1.0".into(), description: "".into(), bottles: vec![], deps: vec!["a".into()], bin: vec![] },
        ]);

        let result = resolve_dependencies(&["a".into()], &index);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Circular dependency"));
    }
}
