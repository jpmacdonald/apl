use crate::indexer::forges::traits::{AssetInfo, ReleaseInfo};
use crate::types::Sha256Digest;
use anyhow::Result;
use std::collections::HashMap;

/// Metadata for a port release, stored in R2 at `ports/<name>/<version>/<arch>.json`
/// or aggregated.
///
/// For the prototype, we assume we can list the bucket prefix `ports/<name>/`
/// and find JSON files that describe the release.
use crate::types::Artifact;

// Removed local PortMetadata struct definition as we now use apl_types::Artifact

/// Fetch release metadata for a port package from the remote R2 bucket.
///
/// Reads the index file at `<bucket_url>/ports/<package_name>/index.json`,
/// groups the artifacts by version, and returns them as [`ReleaseInfo`]
/// entries sorted in descending version order.
///
/// Returns an empty list if the index file is not found (HTTP 404).
///
/// # Errors
///
/// Returns an error if the HTTP request fails with a status other than 404,
/// or if the response body cannot be deserialized.
pub async fn fetch_releases(
    client: &reqwest::Client,
    package_name: &str,
    bucket_url: &str,
) -> Result<Vec<ReleaseInfo>> {
    // Using `apl.pub/ports/<name>/index.json` (The "Repository Metadata" pattern).

    let index_url = format!(
        "{}/ports/{}/index.json",
        bucket_url.trim_end_matches('/'),
        package_name
    );

    // We treat 404 as empty list
    let resp = client.get(&index_url).send().await?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(Vec::new());
    }
    let resp = resp.error_for_status()?;

    let entries: Vec<Artifact> = resp.json().await?;
    println!("    {} port entries for {package_name}", entries.len());

    // Group by version
    let mut by_version: HashMap<String, Vec<Artifact>> = HashMap::new();
    for entry in entries {
        by_version
            .entry(entry.version.clone())
            .or_default()
            .push(entry);
    }

    let mut releases = Vec::new();
    for (version, artifacts) in by_version {
        let assets: Vec<AssetInfo> = artifacts
            .into_iter()
            .map(|a| {
                // Reconstruct a filename-like asset name for the selector to match
                // e.g. "ruby-3.2.2-aarch64-apple-darwin.tar.gz"
                let name = format!("{}-{}-{}.tar.gz", a.name, a.version, a.arch);
                AssetInfo {
                    name,
                    download_url: a.url,
                    digest: Sha256Digest::new(a.sha256).ok(),
                }
            })
            .collect();

        releases.push(ReleaseInfo {
            tag_name: version.clone(),
            prerelease: false,
            prune: false,
            body: "Built by apl-ports factory".to_string(),
            assets,
        });
    }

    // Sort descending by version (heuristic)
    releases.sort_by(|a, b| b.tag_name.cmp(&a.tag_name));

    Ok(releases)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mockito::Server;

    #[tokio::test]
    async fn test_fetch_ports_releases() {
        let mut server = Server::new_async().await;
        let mock_url = server.url();

        let mock_body = r#"[
            {
                "name": "ruby",
                "version": "3.2.2",
                "arch": "aarch64-apple-darwin",
                "url": "https://cdn.example.com/blobs/sha256/123",
                "sha256": "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
            },
            {
                "name": "ruby",
                "version": "3.2.2",
                "arch": "x86_64-apple-darwin",
                "url": "https://cdn.example.com/blobs/sha256/456",
                "sha256": "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890"
            }
        ]"#;

        let _m = server
            .mock("GET", "/ports/ruby/index.json")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(mock_body)
            .create_async()
            .await;

        let client = reqwest::Client::new();
        let releases = fetch_releases(&client, "ruby", &mock_url).await.unwrap();

        assert_eq!(releases.len(), 1); // 1 version
        let r = &releases[0];
        assert_eq!(r.tag_name, "3.2.2");
        assert_eq!(r.assets.len(), 2);

        // Check asset names
        let names: Vec<&str> = r.assets.iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"ruby-3.2.2-aarch64-apple-darwin.tar.gz"));
        assert!(names.contains(&"ruby-3.2.2-x86_64-apple-darwin.tar.gz"));
    }
}
