//! Composition root: builds the live GitHub adapter from a stored token.
//!
//! The token comes from the OS secure store (see
//! [`infrastructure::KeyringSecureStore`]), not the environment. The rest of the
//! app talks to an `Arc<dyn GitHubPort>`, so previews and tests can hand in a
//! fake instead of the real client.

use std::sync::Arc;

use application::{BoardService, GitHubPort};
use domain::{AppResult, GitHubToken, RepoRef, Slice};
use infrastructure::GitHubClient;

/// The repository the v1 desktop app shows. Hardcoded until project selection
/// lands in a later slice.
const REPO_OWNER: &str = "funkode-io";
const REPO_NAME: &str = "zfirot";

/// The app's wired dependencies: a project and the GitHub port behind it.
#[derive(Clone)]
pub struct AppState {
    repo: RepoRef,
    port: Arc<dyn GitHubPort>,
}

impl AppState {
    /// Wire the live GitHub adapter from a stored Personal Access Token.
    ///
    /// The token is read from the OS secure store by the caller; this only turns
    /// it into an authenticated [`GitHubClient`].
    pub fn from_token(token: &GitHubToken) -> AppResult<Self> {
        let client = GitHubClient::new(token.expose())?;
        Ok(Self::with_port(
            RepoRef::new(REPO_OWNER, REPO_NAME),
            Arc::new(client),
        ))
    }

    /// Build a state around an arbitrary port, for previews and tests.
    pub fn with_port(repo: RepoRef, port: Arc<dyn GitHubPort>) -> Self {
        Self { repo, port }
    }

    /// Load the board for the wired project.
    pub async fn load_board(&self) -> AppResult<Vec<Slice>> {
        BoardService::new(self.port.clone())
            .load_board(&self.repo)
            .await
    }
}
