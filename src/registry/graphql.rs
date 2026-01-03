use crate::registry::github::{GithubAsset, GithubRelease};
use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize)]
struct GraphQlQuery {
    query: String,
}

#[derive(Deserialize, Debug)]
struct GraphQlResponse<T> {
    data: Option<T>,
    errors: Option<Vec<GraphQlError>>,
}

#[derive(Deserialize, Debug)]
struct GraphQlError {
    message: String,
}

// Temporary structs for parsing the nested GraphQL structure
#[derive(Deserialize, Debug)]
struct RepositoryData {
    releases: ReleaseConnection,
}

#[derive(Deserialize, Debug)]
struct ReleaseConnection {
    nodes: Vec<ReleaseNode>,
}

#[derive(Deserialize, Debug)]
struct ReleaseNode {
    #[serde(rename = "tagName")]
    tag_name: String,
    #[serde(rename = "isDraft")]
    is_draft: bool,
    #[serde(rename = "isPrerelease")]
    is_prerelease: bool,
    #[serde(rename = "releaseAssets")]
    release_assets: AssetConnection,
    description: Option<String>,
}

#[derive(Deserialize, Debug)]
struct AssetConnection {
    nodes: Vec<AssetNode>,
}

#[derive(Deserialize, Debug)]
struct AssetNode {
    name: String,
    #[serde(rename = "downloadUrl")]
    download_url: String,
    digest: Option<String>,
}

/// Fetch releases for multiple repositories in a single GraphQL request
pub async fn fetch_batch_releases(
    client: &Client,
    token: &str,
    repos: &[(String, String)], // (owner, repo)
) -> Result<HashMap<(String, String), Vec<GithubRelease>>> {
    if repos.is_empty() {
        return Ok(HashMap::new());
    }

    // Build dynamic query with aliases
    // repo_0: repository(owner: "x", name: "y") { ... }
    let mut fragment = String::new();
    for (i, (owner, name)) in repos.iter().enumerate() {
        fragment.push_str(&format!(
            r#"
            repo_{}: repository(owner: "{}", name: "{}") {{
                releases(first: 20, orderBy: {{field: CREATED_AT, direction: DESC}}) {{
                    nodes {{
                        tagName
                        isDraft
                        isDraft
                        isPrerelease
                        description
                        releaseAssets(first: 100) {{
                            nodes {{
                                name
                                downloadUrl
                                digest
                            }}
                        }}
                    }}
                }}
            }}
            "#,
            i, owner, name
        ));
    }

    let query_str = format!("query {{ {} }}", fragment);
    let payload = GraphQlQuery { query: query_str };

    let resp = client
        .post("https://api.github.com/graphql")
        .header("Authorization", format!("Bearer {}", token))
        .header("User-Agent", "apl-pkg")
        .json(&payload)
        .send()
        .await?;

    if !resp.status().is_success() {
        let text = resp.text().await?;
        anyhow::bail!("GraphQL request failed: {}", text);
    }

    // Parse into a dynamic map of "repo_N" -> RepositoryData
    let text = resp.text().await?;
    let raw_body: GraphQlResponse<HashMap<String, Option<RepositoryData>>> =
        serde_json::from_str(&text).map_err(|e| {
            let snippet: String = text.chars().take(500).collect();
            anyhow::anyhow!(
                "Failed to parse GraphQL JSON: {}. Response snippet: {}",
                e,
                snippet
            )
        })?;

    if let Some(errors) = raw_body.errors {
        if !errors.is_empty() {
            // We just log the first error but don't bail completely if some data returned
            eprintln!("GraphQL Warning: {}", errors[0].message);
        }
    }

    let data = raw_body
        .data
        .ok_or_else(|| anyhow::anyhow!("No data in GraphQL response"))?;
    let mut result = HashMap::new();

    for (i, (owner, name)) in repos.iter().enumerate() {
        let key = format!("repo_{}", i);
        if let Some(Some(repo_data)) = data.get(&key) {
            let mut releases = Vec::new();

            for node in &repo_data.releases.nodes {
                let assets = node
                    .release_assets
                    .nodes
                    .iter()
                    .map(|a| GithubAsset {
                        name: a.name.clone(),
                        browser_download_url: a.download_url.clone(),
                        digest: a.digest.clone(),
                    })
                    .collect();

                releases.push(GithubRelease {
                    id: 0, // Not provided by simplified query, not used by logic
                    tag_name: node.tag_name.clone(),
                    draft: node.is_draft,
                    prerelease: node.is_prerelease,
                    body: node.description.clone().unwrap_or_default(),
                    assets,
                });
            }
            result.insert((owner.clone(), name.clone()), releases);
        }
    }

    Ok(result)
}
