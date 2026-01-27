//! PubGrub adapter for APL dependency resolution.
//!
//! Implements the `DependencyProvider` trait to enable SAT-solver based
//! version conflict resolution.

use crate::types::PackageName;
use apl_schema::PackageIndex;
use pubgrub::range::Range;
use pubgrub::solver::{Dependencies, DependencyProvider};
use pubgrub::version::SemanticVersion;
use std::borrow::Borrow;
use std::error::Error;
use std::fmt;

/// A wrapper type for package names that implements `PubGrub`'s `Package` trait.
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct PkgId(
    /// The underlying [`PackageName`].
    pub PackageName,
);

impl fmt::Display for PkgId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Borrow<str> for PkgId {
    fn borrow(&self) -> &str {
        self.0.as_ref()
    }
}

/// Adapter that provides APL package info to the `PubGrub` solver.
///
/// Implements [`DependencyProvider`] so the `PubGrub` SAT-based resolver
/// can query available versions and dependency constraints from the
/// [`PackageIndex`].
#[derive(Debug)]
pub struct AplDependencyProvider<'a> {
    index: &'a PackageIndex,
}

impl<'a> AplDependencyProvider<'a> {
    /// Create a new provider backed by the given index.
    pub fn new(index: &'a PackageIndex) -> Self {
        Self { index }
    }

    /// Parse a version string into `SemanticVersion`.
    fn parse_version(s: &str) -> Option<SemanticVersion> {
        let parts: Vec<&str> = s.split('.').collect();
        if parts.len() >= 2 {
            let major = parts[0].parse().ok()?;
            let minor = parts[1].parse().ok()?;
            let patch = parts.get(2).and_then(|p| p.parse().ok()).unwrap_or(0);
            Some(SemanticVersion::new(major, minor, patch))
        } else {
            None
        }
    }
}

impl DependencyProvider<PkgId, SemanticVersion> for AplDependencyProvider<'_> {
    fn choose_package_version<T: Borrow<PkgId>, U: Borrow<Range<SemanticVersion>>>(
        &self,
        potential_packages: impl Iterator<Item = (T, U)>,
    ) -> Result<(T, Option<SemanticVersion>), Box<dyn Error>> {
        // Pick the first package and find the highest compatible version
        let (pkg, range) = potential_packages
            .into_iter()
            .next()
            .expect("potential_packages is never empty");

        let pkg_name: &PackageName = &pkg.borrow().0;
        let range: &Range<SemanticVersion> = range.borrow();

        let version = self.index.find(pkg_name).and_then(|entry| {
            entry
                .releases
                .iter()
                .filter_map(|r| Self::parse_version(&r.version))
                .filter(|v| range.contains(v))
                .max()
        });

        Ok((pkg, version))
    }

    fn get_dependencies(
        &self,
        pkg: &PkgId,
        version: &SemanticVersion,
    ) -> Result<Dependencies<PkgId, SemanticVersion>, Box<dyn Error>> {
        use pubgrub::solver::DependencyConstraints;

        let pkg_name = &pkg.0;
        let version_str = version.to_string();

        if let Some(entry) = self.index.find(pkg_name) {
            if let Some(release) = entry.releases.iter().find(|r| r.version == version_str) {
                let mut deps: DependencyConstraints<PkgId, SemanticVersion> =
                    DependencyConstraints::default();
                for dep in &release.deps {
                    deps.insert(PkgId(PackageName::new(dep)), Range::any());
                }
                return Ok(Dependencies::Known(deps));
            }
        }

        Ok(Dependencies::Known(DependencyConstraints::default()))
    }
}

/// Resolve dependencies using the `PubGrub` algorithm.
///
/// Returns a sorted list of `(PackageName, version_string)` pairs
/// representing the full dependency solution rooted at `root`.
///
/// # Errors
///
/// Returns an error string if the root package is not found in the index
/// or the solver encounters an unresolvable version conflict.
pub fn resolve_with_pubgrub(
    root: &PackageName,
    index: &PackageIndex,
) -> Result<Vec<(PackageName, String)>, String> {
    let provider = AplDependencyProvider::new(index);
    let root_pkg = PkgId(root.clone());

    // Find root version
    let root_version = index
        .find(root)
        .and_then(|e| e.latest())
        .and_then(|r| AplDependencyProvider::parse_version(&r.version))
        .ok_or_else(|| format!("Package {root} not found"))?;

    match pubgrub::solver::resolve(&provider, root_pkg, root_version) {
        Ok(solution) => {
            let mut result: Vec<(PackageName, String)> = solution
                .into_iter()
                .map(|(pkg, version)| (pkg.0, version.to_string()))
                .collect();
            result.sort_by(|a, b| a.0.cmp(&b.0));
            Ok(result)
        }
        Err(e) => Err(format!("Resolution failed: {e}")),
    }
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

    fn simple_entry(name: &str, version: &str, deps: Vec<String>) -> IndexEntry {
        IndexEntry {
            name: name.into(),
            description: String::new(),
            homepage: String::new(),
            type_: "cli".into(),
            bins: vec![],
            releases: vec![VersionInfo {
                version: version.into(),
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
    fn test_pubgrub_simple_resolution() {
        let index = mock_index(vec![
            simple_entry("a", "1.0.0", vec!["b".into()]),
            simple_entry("b", "2.0.0", vec![]),
        ]);

        let result = resolve_with_pubgrub(&"a".into(), &index);
        assert!(result.is_ok());

        let solution = result.unwrap();
        assert!(solution.iter().any(|(n, _)| n.as_ref() as &str == "a"));
        assert!(solution.iter().any(|(n, _)| n.as_ref() as &str == "b"));
    }

    #[test]
    fn test_pubgrub_no_package() {
        let index = mock_index(vec![]);
        let result = resolve_with_pubgrub(&"nonexistent".into(), &index);
        assert!(result.is_err());
    }
}
