//! Installation Flow Typestate Pattern
//!
//! Models the installation pipeline as a series of explicit state transitions:
//!
//! ```text
//! UnresolvedPackage --[resolve()]--> ResolvedPackage --[prepare()]--> PreparedPackage
//! ```
//!
//! This enforces at compile-time that you cannot prepare a package before resolving it,
//! preventing logic errors where code attempts to extract an archive that hasn't been
//! downloaded yet.
//!
//! # Usage
//!
//! ```ignore
//! use apl::ops::flow::UnresolvedPackage;
//!
//! let unresolved = UnresolvedPackage::new(name, None);
//! let resolved = unresolved.resolve(Some(&index))?;
//! let prepared = resolved.prepare(&client, &reporter).await?;
//! ```

use reqwest::Client;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use tempfile::TempDir;

use crate::core::index::{IndexEntry, PackageIndex, VersionInfo};
use crate::ops::InstallError;
use crate::package::{
    ArtifactFormat, Binary, Dependencies, Hints, InstallSpec, InstallStrategy, Package,
    PackageInfo, PackageType, Source,
};
use crate::ui::Reporter;
use crate::{Arch, PackageName, Version};

/// Represents the source of the artifact to be downloaded or built.
///
/// This enum distinguishes between pre-compiled binaries and source archives,
/// allowing the installation logic to handle them differently (e.g., source
/// packages may require building).
#[derive(Debug, Clone)]
pub enum ArtifactKind {
    /// A pre-compiled binary for the current architecture.
    Binary {
        /// Download URL for the binary archive.
        url: String,
        /// BLAKE3 hash for verification.
        hash: String,
    },
    /// Source code that requires building.
    Source {
        /// Download URL for the source archive.
        url: String,
        /// BLAKE3 hash for verification.
        hash: String,
    },
}

impl ArtifactKind {
    /// Get the download URL for this artifact.
    pub fn url(&self) -> &str {
        match self {
            Self::Binary { url, .. } | Self::Source { url, .. } => url,
        }
    }

    /// Get the BLAKE3 hash for verification.
    pub fn hash(&self) -> &str {
        match self {
            Self::Binary { hash, .. } | Self::Source { hash, .. } => hash,
        }
    }

    /// Returns `true` if this is a source artifact requiring building.
    pub fn is_source(&self) -> bool {
        matches!(self, Self::Source { .. })
    }
}

/// State 1: A package that has been requested but not yet resolved.
///
/// This is the initial state in the installation pipeline. The package name
/// is known, but the version, download URL, and metadata have not yet been
/// determined.
///
/// # Transitions
///
/// - [`resolve()`](Self::resolve) → [`ResolvedPackage`]
pub struct UnresolvedPackage {
    /// The requested package name.
    pub name: PackageName,
    /// Optional requested version (None = latest).
    pub requested: Option<Version>,
}

/// State 2: A package whose version and metadata have been determined.
///
/// At this stage, we know exactly which version to install, where to download
/// the artifact from, and the expected hash for verification.
///
/// # Transitions
///
/// - [`prepare()`](Self::prepare) → [`PreparedPackage`]
pub struct ResolvedPackage {
    /// The resolved package name.
    pub name: PackageName,
    /// The resolved version to install.
    pub version: Version,
    /// Full package definition with metadata.
    pub def: Package,
    /// The artifact (binary or source) to download.
    pub artifact: ArtifactKind,
}

/// State 3: A package that has been downloaded and extracted.
///
/// This is the final state before installation. The archive has been downloaded,
/// verified, and extracted to a temporary directory. The package is ready to be
/// moved to the store and linked.
pub struct PreparedPackage {
    /// The resolved package information.
    pub resolved: ResolvedPackage,
    /// Path to the extracted contents.
    pub extracted_path: PathBuf,
    /// List of binaries to install.
    pub bin_list: Vec<String>,
    /// Temporary directory (cleaned up on drop).
    pub temp_dir: TempDir,
}

impl UnresolvedPackage {
    /// Create a new unresolved package request.
    ///
    /// # Arguments
    ///
    /// * `name` - The package name to install
    /// * `requested` - Optional specific version (None = latest)
    pub fn new(name: PackageName, requested: Option<Version>) -> Self {
        Self { name, requested }
    }

    /// Resolves the package against the provided index or the local filesystem.
    pub fn resolve(self, index: Option<&PackageIndex>) -> Result<ResolvedPackage, InstallError> {
        let package_path = Path::new(&*self.name);

        if package_path.exists() {
            Self::resolve_from_file(package_path)
        } else {
            Self::resolve_from_index(&self.name, self.requested.as_ref(), index)
        }
    }

    /// Resolves a package from a local `.toml` file.
    fn resolve_from_file(path: &Path) -> Result<ResolvedPackage, InstallError> {
        let package_def =
            Package::from_file(path).map_err(|e| InstallError::Validation(e.to_string()))?;

        if let Some(bottle) = package_def.binary_for_current_arch() {
            Ok(ResolvedPackage {
                name: package_def.package.name.clone(),
                version: package_def.package.version.clone(),
                artifact: ArtifactKind::Binary {
                    url: bottle.url.clone(),
                    hash: bottle.blake3.clone(),
                },
                def: package_def,
            })
        } else if !package_def.source.url.is_empty() {
            Ok(ResolvedPackage {
                name: package_def.package.name.clone(),
                version: package_def.package.version.clone(),
                artifact: ArtifactKind::Source {
                    url: package_def.source.url.clone(),
                    hash: package_def.source.blake3.clone(),
                },
                def: package_def,
            })
        } else {
            Err(InstallError::Validation(format!(
                "Package {} has no binary for this arch and no source.",
                path.display()
            )))
        }
    }

    /// Resolves a package from the index.
    fn resolve_from_index(
        name: &PackageName,
        requested: Option<&Version>,
        index: Option<&PackageIndex>,
    ) -> Result<ResolvedPackage, InstallError> {
        let index_ref = index.ok_or_else(|| {
            InstallError::Validation(format!("Index missing, cannot find {name}"))
        })?;

        let entry = index_ref
            .find(name)
            .ok_or_else(|| InstallError::Validation(format!("Package {name} not found")))?;

        let release = Self::select_release(name, requested, entry)?;
        let (artifact, current_arch) = Self::select_artifact(name, release)?;
        let package_def = Self::build_synthetic_package(entry, release, &artifact, current_arch);

        Ok(ResolvedPackage {
            name: package_def.package.name.clone(),
            version: package_def.package.version.clone(),
            def: package_def,
            artifact,
        })
    }

    /// Selects the appropriate release (version) from the index entry.
    fn select_release<'a>(
        name: &PackageName,
        requested: Option<&Version>,
        entry: &'a IndexEntry,
    ) -> Result<&'a VersionInfo, InstallError> {
        if let Some(v) = requested {
            if v == "latest" {
                entry.latest().ok_or_else(|| {
                    InstallError::Validation(format!("No releases found for {name}"))
                })
            } else {
                entry
                    .find_version(v)
                    .ok_or_else(|| InstallError::Validation(format!("Version {v} not found")))
            }
        } else {
            entry
                .latest()
                .ok_or_else(|| InstallError::Validation(format!("No releases found for {name}")))
        }
    }

    /// Selects either a binary or source artifact from the release.
    fn select_artifact(
        name: &PackageName,
        release: &VersionInfo,
    ) -> Result<(ArtifactKind, Arch), InstallError> {
        let current_arch = Arch::current();
        let bin_artifact = release
            .binaries
            .iter()
            .find(|b| Arch::from_str(&b.arch).ok() == Some(current_arch));

        if let Some(b) = bin_artifact {
            Ok((
                ArtifactKind::Binary {
                    url: b.url.clone(),
                    hash: b.blake3.clone(),
                },
                current_arch,
            ))
        } else if let Some(src) = &release.source {
            Ok((
                ArtifactKind::Source {
                    url: src.url.clone(),
                    hash: src.blake3.clone(),
                },
                current_arch,
            ))
        } else {
            Err(InstallError::Validation(format!(
                "No binary/source available for {name} on {current_arch}"
            )))
        }
    }

    /// Builds a synthetic `Package` definition from index data for installation.
    fn build_synthetic_package(
        entry: &IndexEntry,
        release: &VersionInfo,
        artifact: &ArtifactKind,
        current_arch: Arch,
    ) -> Package {
        let mut binary_map = std::collections::HashMap::new();
        if let ArtifactKind::Binary { url, hash } = artifact {
            binary_map.insert(
                current_arch,
                Binary {
                    url: url.clone(),
                    blake3: hash.clone(),
                    format: ArtifactFormat::Binary,
                    arch: current_arch,
                    macos: "11.0".to_string(),
                },
            );
        }

        Package {
            package: PackageInfo {
                name: PackageName::from(entry.name.clone()),
                version: Version::from(release.version.clone()),
                description: entry.description.clone(),
                homepage: String::new(),
                license: String::new(),
                type_: if entry.type_ == "app" {
                    PackageType::App
                } else {
                    PackageType::Cli
                },
            },
            source: Source {
                url: if artifact.is_source() {
                    artifact.url().to_string()
                } else {
                    String::new()
                },
                blake3: if artifact.is_source() {
                    artifact.hash().to_string()
                } else {
                    String::new()
                },
                format: ArtifactFormat::TarGz,
                strip_components: 1,
            },
            binary: binary_map,
            dependencies: Dependencies {
                runtime: release.deps.clone(),
                build: release.build_deps.clone(),
                optional: vec![],
            },
            install: InstallSpec {
                strategy: if entry.type_ == "app" {
                    InstallStrategy::App
                } else {
                    InstallStrategy::Link
                },
                bin: if release.bin.is_empty() {
                    vec![entry.name.clone()]
                } else {
                    release.bin.clone()
                },
                lib: vec![],
                include: vec![],
                script: String::new(),
                app: release.app.clone(),
            },
            hints: Hints {
                post_install: release.hints.clone(),
            },
            build: if artifact.is_source() {
                Some(crate::package::BuildSpec {
                    dependencies: release.build_deps.clone(),
                    script: release.build_script.clone(),
                })
            } else {
                None
            },
        }
    }
}

impl ResolvedPackage {
    /// Downloads and extracts the package artifact.
    pub async fn prepare<R: Reporter + Clone + 'static>(
        self,
        client: &Client,
        reporter: &R,
    ) -> Result<PreparedPackage, InstallError> {
        let tmp_path = crate::tmp_path();
        std::fs::create_dir_all(&tmp_path).map_err(InstallError::Io)?;
        let temp_dir = tempfile::Builder::new()
            .prefix("apl-")
            .tempdir_in(tmp_path)
            .map_err(InstallError::Io)?;

        let pkg_format = match &self.artifact {
            ArtifactKind::Source { .. } => self.def.source.format.clone(),
            ArtifactKind::Binary { .. } => self
                .def
                .binary_for_current_arch()
                .map(|b| b.format.clone())
                .unwrap_or(ArtifactFormat::Binary),
        };

        let strategy = self.def.install.strategy.clone();
        let is_dmg = (strategy == InstallStrategy::App || strategy == InstallStrategy::Pkg)
            && (pkg_format == ArtifactFormat::Dmg
                || self.artifact.url().to_lowercase().ends_with(".dmg")
                || self.artifact.url().to_lowercase().ends_with(".pkg"));

        let download_or_extract_path: PathBuf;

        if is_dmg {
            let dest_file = temp_dir.path().join(
                self.artifact
                    .url()
                    .split('/')
                    .next_back()
                    .unwrap_or("pkg.dmg"),
            );
            crate::io::download::DownloadRequest::new(
                client,
                &self.name,
                &self.version,
                self.artifact.url(),
                &dest_file,
                self.artifact.hash(),
                reporter,
            )
            .execute()
            .await?;
            download_or_extract_path = dest_file;
        } else {
            let cache_file = crate::cache_path().join(self.artifact.hash());
            if let Some(p) = cache_file.parent() {
                std::fs::create_dir_all(p).ok();
            }

            let extract_dir = temp_dir.path().join("extracted");
            std::fs::create_dir_all(&extract_dir).map_err(InstallError::Io)?;

            crate::io::download::DownloadRequest::new(
                client,
                &self.name,
                &self.version,
                self.artifact.url(),
                &cache_file,
                self.artifact.hash(),
                reporter,
            )
            .with_extract_dest(&extract_dir)
            .execute()
            .await?;

            download_or_extract_path = extract_dir;
            if self.artifact.is_source() && self.def.source.strip_components > 0 {
                crate::io::extract::strip_components(&download_or_extract_path)
                    .map_err(|e| InstallError::Other(e.to_string()))?;
            }
        }

        Ok(PreparedPackage {
            bin_list: self.def.install.bin.clone(),
            resolved: self,
            extracted_path: download_or_extract_path,
            temp_dir,
        })
    }
}
