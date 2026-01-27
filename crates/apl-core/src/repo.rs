/// A validated GitHub repository reference in `owner/repo` format.
///
/// # Example
///
/// ```
/// use apl_core::repo::GitHubRepo;
///
/// let repo = GitHubRepo::new("jqlang/jq").unwrap();
/// assert_eq!(repo.owner(), "jqlang");
/// assert_eq!(repo.name(), "jq");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct GitHubRepo(String);

impl GitHubRepo {
    /// Create a new `GitHubRepo`, validating the `owner/repo` format.
    ///
    /// # Errors
    ///
    /// Returns an error string if `s` is not in `owner/repo` format or if
    /// either component is empty.
    pub fn new(s: &str) -> Result<Self, String> {
        if s.contains('/') && s.split('/').count() == 2 {
            let parts: Vec<&str> = s.split('/').collect();
            if !parts[0].is_empty() && !parts[1].is_empty() {
                return Ok(Self(s.to_string()));
            }
        }
        Err(format!(
            "Invalid GitHub repo format: expected 'owner/repo', got '{s}'"
        ))
    }

    /// Get the owner part.
    pub fn owner(&self) -> &str {
        self.0.split('/').next().unwrap_or("")
    }

    /// Get the repo name part.
    pub fn name(&self) -> &str {
        self.0.split('/').nth(1).unwrap_or("")
    }

    /// Return the raw `owner/repo` string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for GitHubRepo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A repository key uniquely identifying a GitHub repository
///
/// This newtype eliminates the ambiguity of `(String, String)` tuples
/// and makes the API self-documenting.
#[derive(Debug, Clone, Hash, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RepoKey {
    /// Repository owner (GitHub user or organization).
    pub owner: String,
    /// Repository name.
    pub repo: String,
}

impl RepoKey {
    /// Create a new `RepoKey` from an owner and repository name.
    pub fn new(owner: impl Into<String>, repo: impl Into<String>) -> Self {
        Self {
            owner: owner.into(),
            repo: repo.into(),
        }
    }

    /// Create a `RepoKey` from an existing [`GitHubRepo`].
    pub fn from_github_repo(gh: &GitHubRepo) -> Self {
        Self {
            owner: gh.owner().to_string(),
            repo: gh.name().to_string(),
        }
    }

    /// Convert to a tuple (for compatibility with existing code)
    pub fn to_tuple(&self) -> (String, String) {
        (self.owner.clone(), self.repo.clone())
    }
}

impl std::fmt::Display for RepoKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.owner, self.repo)
    }
}
