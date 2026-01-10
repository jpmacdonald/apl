use super::{GithubAsset, GithubRelease};
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
    #[serde(default)]
    digest: Option<String>,
}

/// Escape special characters in GraphQL string literals
fn escape_graphql_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

/// Generate a consistent alias for a repository in GraphQL queries
#[inline]
fn repo_alias(index: usize) -> String {
    format!("repo_{index}")
}

/// Fetch releases for multiple repositories in a single GraphQL request
pub async fn fetch_batch_releases(
    client: &Client,
    token: &str,
    repos: &[crate::types::RepoKey],
) -> Result<HashMap<crate::types::RepoKey, Vec<GithubRelease>>> {
    if repos.is_empty() {
        return Ok(HashMap::new());
    }

    // Build dynamic query with aliases
    // repo_0: repository(owner: "x", name: "y") { ... }
    let mut fragment = String::new();
    for (i, key) in repos.iter().enumerate() {
        fragment.push_str(&format!(
            r#"
            {}: repository(owner: "{}", name: "{}") {{
                releases(first: 20, orderBy: {{field: CREATED_AT, direction: DESC}}) {{
                    nodes {{
                        tagName
                        isDraft
                        isPrerelease
                        description
                        releaseAssets(first: 30) {{
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
            repo_alias(i),
            escape_graphql_string(&key.owner),
            escape_graphql_string(&key.repo)
        ));
    }

    let query_str = format!("query {{ {fragment} }}");
    let payload = GraphQlQuery { query: query_str };

    if token.is_empty() {
        anyhow::bail!(
            "GITHUB_TOKEN is missing or empty. Indexing requires partial auth to fetch releases via GraphQL."
        );
    }

    let mut attempt = 0;
    while attempt < 3 {
        attempt += 1;
        let resp_result = client
            .post("https://api.github.com/graphql")
            .header("Authorization", format!("Bearer {token}"))
            .header("User-Agent", "apl-pkg")
            .json(&payload)
            .send()
            .await;

        match resp_result {
            Ok(resp) => {
                if resp.status().is_success() {
                    let text = resp.text().await?;
                    // Check for "couldn't respond in time" in the body even if status is 200 (GraphQL quirk)
                    if text.contains("couldn't respond to your request in time") {
                        if attempt < 3 {
                            println!("   ⚠ GitHub timeout, retrying ({attempt}/3)...");
                            tokio::time::sleep(tokio::time::Duration::from_millis(1000 * attempt))
                                .await;
                            continue;
                        } else {
                            anyhow::bail!("GraphQL request failed after retries: timeout");
                        }
                    }

                    // Parse the success response
                    let raw_body: GraphQlResponse<HashMap<String, Option<RepositoryData>>> =
                        serde_json::from_str(&text).map_err(|e| {
                            let snippet: String = text.chars().take(500).collect();
                            anyhow::anyhow!("Failed to parse JSON: {e}. Snippet: {snippet}")
                        })?;

                    if let Some(ref errors) = raw_body.errors {
                        if !errors.is_empty() {
                            for err in errors {
                                println!("   ⚠ GraphQL Error: {}", err.message);
                            }
                        }
                    }

                    // Clone errors if needed for the error message, or just print them above
                    let data = raw_body.data.ok_or_else(|| {
                        anyhow::anyhow!("No data in response (errors: {:?})", raw_body.errors)
                    })?;

                    // Debug: mismatched keys?
                    // println!("   Debug: Response keys: {:?}", data.keys().collect::<Vec<_>>());

                    let mut result = HashMap::new();
                    for (i, key) in repos.iter().enumerate() {
                        let alias = repo_alias(i);
                        match data.get(&alias) {
                            Some(Some(repo_data)) => {
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
                                        id: 0,
                                        tag_name: node.tag_name.clone(),
                                        draft: node.is_draft,
                                        prerelease: node.is_prerelease,
                                        body: node.description.clone(),
                                        assets,
                                    });
                                }
                                result.insert(key.clone(), releases);
                            }
                            Some(None) => {
                                eprintln!(
                                    "   ⚠ Repo found but data is null: {} (alias: {})",
                                    key, alias
                                );
                            }
                            None => {
                                eprintln!(
                                    "   ⚠ Repo missing from response: {} (alias: {})",
                                    key, alias
                                );
                            }
                        }
                    }
                    return Ok(result);
                } else if resp.status().is_server_error() && attempt < 3 {
                    eprintln!(
                        "   ⚠ GitHub server error {}, retrying ({attempt}/3)...",
                        resp.status()
                    );
                    tokio::time::sleep(tokio::time::Duration::from_millis(1000 * attempt)).await;
                    continue;
                }

                // If we get here, it's a non-retriable error or we exhausted retries
                let text = resp.text().await?;
                anyhow::bail!("GraphQL request failed: {text}");
            }
            Err(e) => {
                if attempt < 3 {
                    eprintln!("   ⚠ Network error: {e}, retrying ({attempt}/3)...");
                    tokio::time::sleep(tokio::time::Duration::from_millis(1000 * attempt)).await;
                    continue;
                }
                anyhow::bail!("GraphQL request failed: {e}");
            }
        }
    }

    anyhow::bail!("Request failed after 3 attempts")
}

/// Fetch only the latest release tag for multiple repositories.
/// This is a lightweight query for delta checking (no assets, no bodies).
pub async fn fetch_latest_versions_batch(
    client: &Client,
    token: &str,
    repos: &[crate::types::RepoKey],
) -> Result<HashMap<crate::types::RepoKey, Option<String>>> {
    if repos.is_empty() {
        return Ok(HashMap::new());
    }

    // Build ultra-lightweight query: only tagName of the first release
    let mut fragment = String::new();
    for (i, key) in repos.iter().enumerate() {
        fragment.push_str(&format!(
            r#"
            {}: repository(owner: "{}", name: "{}") {{
                releases(first: 1, orderBy: {{field: CREATED_AT, direction: DESC}}) {{
                    nodes {{
                        tagName
                    }}
                }}
            }}
            "#,
            repo_alias(i),
            escape_graphql_string(&key.owner),
            escape_graphql_string(&key.repo)
        ));
    }

    let query_str = format!("query {{ {fragment} }}");
    let payload = GraphQlQuery { query: query_str };

    // Response structure for lightweight query
    #[derive(Deserialize, Debug)]
    struct LightweightRepoData {
        releases: LightweightReleaseConnection,
    }

    #[derive(Deserialize, Debug)]
    struct LightweightReleaseConnection {
        nodes: Vec<LightweightReleaseNode>,
    }

    #[derive(Deserialize, Debug)]
    struct LightweightReleaseNode {
        #[serde(rename = "tagName")]
        tag_name: String,
    }

    let mut attempt = 0;
    while attempt < 3 {
        attempt += 1;
        let resp_result = client
            .post("https://api.github.com/graphql")
            .header("Authorization", format!("Bearer {token}"))
            .header("User-Agent", "apl-pkg")
            .json(&payload)
            .send()
            .await;

        match resp_result {
            Ok(resp) => {
                if resp.status().is_success() {
                    let text = resp.text().await?;

                    if text.contains("couldn't respond to your request in time") {
                        if attempt < 3 {
                            tokio::time::sleep(tokio::time::Duration::from_millis(500 * attempt))
                                .await;
                            continue;
                        } else {
                            anyhow::bail!("GraphQL request failed after retries: timeout");
                        }
                    }

                    let raw_body: GraphQlResponse<HashMap<String, Option<LightweightRepoData>>> =
                        serde_json::from_str(&text)
                            .map_err(|e| anyhow::anyhow!("Failed to parse GraphQL JSON: {e}"))?;

                    let data = raw_body
                        .data
                        .ok_or_else(|| anyhow::anyhow!("No data in GraphQL response"))?;

                    let mut result = HashMap::new();
                    for (i, key) in repos.iter().enumerate() {
                        let alias = repo_alias(i);
                        let latest_tag = data
                            .get(&alias)
                            .and_then(|opt| opt.as_ref())
                            .and_then(|repo_data| repo_data.releases.nodes.first())
                            .map(|node| node.tag_name.clone());
                        result.insert(key.clone(), latest_tag);
                    }
                    return Ok(result);
                } else if resp.status().is_server_error() && attempt < 3 {
                    tokio::time::sleep(tokio::time::Duration::from_millis(500 * attempt)).await;
                    continue;
                }
                let text = resp.text().await?;
                anyhow::bail!("GraphQL request failed: {text}");
            }
            Err(e) => {
                if attempt < 3 {
                    tokio::time::sleep(tokio::time::Duration::from_millis(500 * attempt)).await;
                    continue;
                }
                anyhow::bail!("GraphQL request failed: {e}");
            }
        }
    }

    anyhow::bail!("Request failed after 3 attempts")
}
