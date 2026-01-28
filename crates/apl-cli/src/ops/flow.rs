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
//! use crate::ops::flow::UnresolvedPackage;
//!
//! let unresolved = UnresolvedPackage::new(name, None);
//! let resolved = unresolved.resolve(Some(&index))?;
//! let prepared = resolved.prepare(&client, &reporter).await?;
//! ```

use reqwest::Client;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

use crate::ops::InstallError;
use crate::ui::Reporter;
use apl_core::package::{
    ArtifactFormat, Dependencies, Hints, InstallSpec, InstallStrategy, Package, PackageInfo,
    PackageType, Source,
};
use apl_schema::index::{IndexEntry, PackageIndex, VersionInfo};
use apl_schema::{
    Arch,
    types::{PackageName, Version},
};

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
        /// Mirror URL (preferred, from artifact store).
        mirror_url: Option<String>,
        /// SHA256 hash for verification.
        hash: String,
    },
    /// Source code that requires building.
    Source {
        /// Download URL for the source archive.
        url: String,
        /// Mirror URL (preferred, from artifact store).
        mirror_url: Option<String>,
        /// SHA256 hash for verification.
        hash: String,
    },
}

impl ArtifactKind {
    /// Get the download URL for this artifact.
    ///
    /// Prefers the mirror URL if available.
    pub fn url(&self) -> &str {
        match self {
            Self::Binary {
                mirror_url: Some(m),
                ..
            }
            | Self::Source {
                mirror_url: Some(m),
                ..
            } => m,
            Self::Binary { url, .. } | Self::Source { url, .. } => url,
        }
    }

    /// Get the upstream (original) URL for this artifact.
    pub fn upstream_url(&self) -> &str {
        match self {
            Self::Binary { url, .. } | Self::Source { url, .. } => url,
        }
    }

    /// Get the SHA256 hash for verification.
    pub fn hash(&self) -> &str {
        match self {
            Self::Binary { hash, .. } | Self::Source { hash, .. } => hash,
        }
    }

    /// Returns `true` if this is a source artifact requiring building.
    pub fn is_source(&self) -> bool {
        matches!(self, Self::Source { .. })
    }

    /// Returns `true` if there's a fallback URL available (mirror differs from upstream).
    pub fn has_fallback(&self) -> bool {
        self.url() != self.upstream_url()
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
/// - [`resolve()`](Self::resolve) -> [`ResolvedPackage`]
#[derive(Debug)]
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
/// - [`prepare()`](Self::prepare) -> [`PreparedPackage`]
#[derive(Debug)]
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
#[derive(Debug)]
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

        if package_def.source.url.is_empty() {
            Err(InstallError::Validation(format!(
                "Package {} has no binary for this arch and no source.",
                path.display()
            )))
        } else {
            Ok(ResolvedPackage {
                name: package_def.package.name.clone(),
                version: package_def.package.version.clone(),
                artifact: ArtifactKind::Source {
                    url: package_def.source.url.clone(),
                    mirror_url: None,
                    hash: package_def.source.sha256.clone(),
                },
                def: package_def,
            })
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
        let (artifact, current_arch) =
            Self::select_artifact(name, release, index_ref.mirror_base_url.as_deref())?;
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
        mirror_base_url: Option<&str>,
    ) -> Result<(ArtifactKind, Arch), InstallError> {
        let current_arch = Arch::current();
        let bin_artifact = release
            .binaries
            .iter()
            .find(|b| b.arch == current_arch || b.arch == Arch::Universal);

        if let Some(b) = bin_artifact {
            let mirror_url = mirror_base_url.map(|base| format!("{base}/cas/{}", b.hash));
            Ok((
                ArtifactKind::Binary {
                    url: b.url.clone(),
                    mirror_url,
                    hash: b.hash.to_string(),
                },
                current_arch,
            ))
        } else if let Some(src) = &release.source {
            let mirror_url = mirror_base_url.map(|base| format!("{base}/cas/{}", src.hash));
            Ok((
                ArtifactKind::Source {
                    url: src.url.clone(),
                    mirror_url,
                    hash: src.hash.to_string(),
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
        _current_arch: Arch,
    ) -> Package {
        // Binary map is no longer used in the new schema, but we still build a synthetic package.

        Package {
            package: PackageInfo {
                name: PackageName::from(entry.name.clone()),
                version: Version::from(release.version.clone()),
                description: entry.description.clone(),
                homepage: String::new(),
                license: String::new(),
                tags: vec![],
                type_: if entry.type_ == "app" {
                    Some(PackageType::App)
                } else {
                    Some(PackageType::Cli)
                },
            },
            source: Source {
                url: if artifact.is_source() {
                    artifact.url().to_string()
                } else {
                    String::new()
                },
                sha256: if artifact.is_source() {
                    artifact.hash().to_string()
                } else {
                    String::new()
                },
                format: ArtifactFormat::TarGz,
                strip_components: Some(1),
            },
            dependencies: Dependencies {
                runtime: release.deps.clone(),
                build: release.build_deps.clone(),
                optional: vec![],
            },
            install: InstallSpec {
                strategy: if entry.type_ == "app" {
                    Some(InstallStrategy::App)
                } else {
                    Some(InstallStrategy::Link)
                },
                bin: Some(if release.bin.is_empty() {
                    vec![entry.name.clone()]
                } else {
                    release.bin.clone()
                }),
                lib: vec![],
                include: vec![],
                script: Some(String::new()),
                app: release.app.clone(),
            },
            hints: Hints {
                post_install: release.hints.clone(),
            },
            build: if artifact.is_source() {
                Some(apl_core::package::BuildSpec {
                    dependencies: release.build_deps.clone(),
                    script: release.build_script.clone(),
                    tag_pattern: String::new(),
                    version_pattern: None,
                    download_url_template: None,
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
            ArtifactKind::Source { .. } => self.def.source.format,
            ArtifactKind::Binary { .. } => {
                // Infer format from URL since it's not explicitly in ArtifactKind yet
                let url = self.artifact.url().to_lowercase();
                let url_path = std::path::Path::new(url.as_str());
                if url.ends_with(".tar.gz") || url_path.extension().is_some_and(|ext| ext == "tgz")
                {
                    ArtifactFormat::TarGz
                } else if url_path.extension().is_some_and(|ext| ext == "zip") {
                    ArtifactFormat::Zip
                } else if url_path.extension().is_some_and(|ext| ext == "dmg") {
                    ArtifactFormat::Dmg
                } else if url_path.extension().is_some_and(|ext| ext == "pkg") {
                    ArtifactFormat::Pkg
                } else {
                    ArtifactFormat::Binary
                }
            }
        };

        let strategy = self
            .def
            .install
            .strategy
            .clone()
            .unwrap_or(InstallStrategy::Link);
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

            // Try primary URL (mirror if available), fall back to upstream on 404
            let download_result = apl_core::io::download::DownloadRequest::new(
                client,
                &self.name,
                &self.version,
                self.artifact.url(),
                &dest_file,
                self.artifact.hash(),
                reporter,
            )
            .execute()
            .await;

            match download_result {
                Ok(_) => {}
                Err(apl_core::io::download::DownloadError::Http(e))
                    if self.artifact.has_fallback()
                        && e.status() == Some(reqwest::StatusCode::NOT_FOUND) =>
                {
                    tracing::info!(
                        "Mirror returned 404, falling back to upstream: {}",
                        self.artifact.upstream_url()
                    );
                    apl_core::io::download::DownloadRequest::new(
                        client,
                        &self.name,
                        &self.version,
                        self.artifact.upstream_url(),
                        &dest_file,
                        self.artifact.hash(),
                        reporter,
                    )
                    .execute()
                    .await?;
                }
                Err(e) => return Err(e.into()),
            }

            download_or_extract_path = dest_file;
        } else {
            let cache_file = crate::cache_path().join(self.artifact.hash());
            if let Some(p) = cache_file.parent() {
                std::fs::create_dir_all(p).ok();
            }

            let extract_dir = temp_dir.path().join("extracted");
            std::fs::create_dir_all(&extract_dir).map_err(InstallError::Io)?;

            // Try primary URL (mirror if available), fall back to upstream on 404
            let download_result = apl_core::io::download::DownloadRequest::new(
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
            .await;

            match download_result {
                Ok(_) => {}
                Err(apl_core::io::download::DownloadError::Http(e))
                    if self.artifact.has_fallback()
                        && e.status() == Some(reqwest::StatusCode::NOT_FOUND) =>
                {
                    tracing::info!(
                        "Mirror returned 404, falling back to upstream: {}",
                        self.artifact.upstream_url()
                    );
                    apl_core::io::download::DownloadRequest::new(
                        client,
                        &self.name,
                        &self.version,
                        self.artifact.upstream_url(),
                        &cache_file,
                        self.artifact.hash(),
                        reporter,
                    )
                    .with_extract_dest(&extract_dir)
                    .execute()
                    .await?;
                }
                Err(e) => return Err(e.into()),
            }

            download_or_extract_path = extract_dir;
            if self.artifact.is_source() && self.def.source.strip_components.unwrap_or(0) > 0 {
                apl_core::io::extract::strip_components(&download_or_extract_path)
                    .map_err(|e| InstallError::Other(e.to_string()))?;
            }
        }

        Ok(PreparedPackage {
            bin_list: self.def.install.effective_bin(&self.name),
            resolved: self,
            extracted_path: download_or_extract_path,
            temp_dir,
        })
    }
}
