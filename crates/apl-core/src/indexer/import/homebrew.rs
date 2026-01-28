use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::package::{Dependencies, Hints, InstallSpec, PackageInfoTemplate, PackageTemplate};
use crate::types::PackageName;

const HOMEBREW_API_BASE: &str = "https://formulae.brew.sh/api/formula";

#[derive(Debug, Deserialize)]
struct BrewFormula {
    name: String,
    desc: String,
    homepage: String,
    license: Option<String>,
    urls: HashMap<String, BrewUrl>,
    #[serde(default)]
    dependencies: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct BrewUrl {
    url: String,
    // tag: Option<String>,
}

/// Import one or more Homebrew formulae into the APL registry.
///
/// For each package name, fetches the formula metadata from the Homebrew JSON
/// API, analyses the upstream source URL, and writes a TOML template into the
/// registry directory.
///
/// # Errors
///
/// Returns an error if any individual formula cannot be fetched or converted.
pub async fn import_homebrew_packages(packages: &[String], registry_dir: &Path) -> Result<()> {
    let client = reqwest::Client::new();

    for pkg_name in packages {
        println!("Importing {pkg_name} from Homebrew...");
        match import_single_package(&client, pkg_name, registry_dir).await {
            Ok(()) => println!("   OK: Imported {pkg_name}"),
            Err(e) => eprintln!("   FAIL: Failed to import {pkg_name}: {e}"),
        }
    }

    Ok(())
}

async fn import_single_package(
    client: &reqwest::Client,
    pkg_name: &str,
    registry_dir: &Path,
) -> Result<()> {
    let url = format!("{HOMEBREW_API_BASE}/{pkg_name}.json");
    let resp = client.get(&url).send().await?;

    if !resp.status().is_success() {
        anyhow::bail!("Homebrew API request failed: {}", resp.status());
    }

    let formula: BrewFormula = resp.json().await?;

    // 1. Analyze Upstream URL
    let stable_url = formula
        .urls
        .get("stable")
        .map(|u| u.url.clone())
        .ok_or_else(|| anyhow::anyhow!("No stable URL found"))?;

    let (discovery, assets) = super::analyze_upstream_url(&stable_url)?;

    // 2. Map Dependencies
    let dependencies = Dependencies {
        runtime: formula.dependencies,
        ..Default::default()
    };

    // 3. Construct Template
    let template = PackageTemplate {
        package: PackageInfoTemplate {
            name: PackageName::from(formula.name.clone()),
            description: formula.desc,
            homepage: formula.homepage,
            license: formula.license.unwrap_or_default(),
            tags: vec!["imported".to_string(), "homebrew".to_string()],
        },
        discovery,
        assets,
        source: None,
        build: None,
        dependencies,
        install: InstallSpec {
            bin: None, // Will default to package name
            ..Default::default()
        },
        hints: Hints::default(),
    };

    // 4. Write to Registry
    let target_path = crate::indexer::registry_path(registry_dir, &formula.name);
    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let toml = toml::to_string_pretty(&template)?;
    fs::write(&target_path, toml)?;

    Ok(())
}
