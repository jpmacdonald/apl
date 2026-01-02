//! Unified registry management logic
pub mod github;

use anyhow::Result;
use reqwest::header;

/// Build an authenticated GitHub client
pub fn build_github_client(token: Option<&str>) -> Result<reqwest::Client> {
    let mut headers = header::HeaderMap::new();
    headers.insert(
        header::USER_AGENT,
        header::HeaderValue::from_static("apl-package-manager"),
    );

    if let Some(t) = token {
        headers.insert(
            header::AUTHORIZATION,
            header::HeaderValue::from_str(&format!("Bearer {t}"))?,
        );
    }

    Ok(reqwest::Client::builder()
        .default_headers(headers)
        .build()?)
}
