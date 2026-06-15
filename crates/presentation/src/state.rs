//! Composition root: builds the live GitHub adapter from a stored token.
//!
//! The token comes from the OS secure store (see
//! [`infrastructure::KeyringSecureStore`]), not the environment. The rest of the
//! app talks to an `Arc<dyn GitHubPort>`, so previews and tests can hand in a
//! fake instead of the real client.

use std::sync::Arc;

use application::{
    BoardService, GitHubPort, LastOpenedService, ProjectStorePort, ProjectsService, SecureStorePort,
};
use domain::{AppAction, AppResult, GitHubToken, Project, RepoRef, Slice};
#[cfg(debug_assertions)]
use infrastructure::EnvSecureStore;
use infrastructure::{FileProjectStore, GitHubClient, KeyringSecureStore};

/// The secure store the running app authenticates against.
///
/// In debug builds, when `ZFIROT_GITHUB_TOKEN` is set the token is read from
/// the environment (see [`EnvSecureStore`]) so repeated `dx serve` rebuilds do
/// not re-trigger the OS keychain prompt. Otherwise — and always in release
/// builds — the OS secure store (keyring) is used.
pub fn secure_store() -> Arc<dyn SecureStorePort> {
    #[cfg(debug_assertions)]
    if EnvSecureStore::is_configured() {
        return Arc::new(EnvSecureStore::from_env());
    }
    Arc::new(KeyringSecureStore::new())
}

/// The on-device store remembering which project was last opened.
pub fn project_store() -> AppResult<Arc<dyn ProjectStorePort>> {
    Ok(Arc::new(FileProjectStore::new()?))
}

/// The projects use-case (recent projects) wired from a stored Personal Access
/// Token. Listing repositories needs GitHub, so it needs a token.
pub fn projects_from_token(token: &GitHubToken) -> AppResult<ProjectsService<Arc<dyn GitHubPort>>> {
    let port: Arc<dyn GitHubPort> = Arc::new(GitHubClient::new(token.expose())?);
    Ok(ProjectsService::new(port))
}

/// The last-opened use-cases, wired from the on-device project store. Purely
/// local persistence: no token or network involved.
fn last_opened_service() -> AppResult<LastOpenedService<Arc<dyn ProjectStorePort>>> {
    Ok(LastOpenedService::new(project_store()?))
}

/// The app's wired dependencies: a project and the GitHub port behind it.
#[derive(Clone)]
pub struct AppState {
    repo: RepoRef,
    port: Arc<dyn GitHubPort>,
}

impl AppState {
    /// Wire the live GitHub adapter from a stored Personal Access Token, scoped
    /// to a chosen repository.
    ///
    /// The token is read from the OS secure store by the caller; this only turns
    /// it into an authenticated [`GitHubClient`].
    pub fn from_token(token: &GitHubToken, repo: RepoRef) -> AppResult<Self> {
        let client = GitHubClient::new(token.expose())?;
        Ok(Self::with_port(repo, Arc::new(client)))
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

/// The accessible projects, most-recently-pushed first, for the home screen.
pub async fn recent_projects(token: &GitHubToken) -> AppResult<Vec<Project>> {
    projects_from_token(token)?.recent_projects().await
}

/// The project to reopen on launch, or `None` to show the home screen. A local
/// store read: no token needed.
pub async fn last_opened() -> AppResult<Option<RepoRef>> {
    last_opened_service()?.last_opened().await
}

/// Remember `repo` as the last-opened project, so the next launch reopens it.
/// A local store write only: no token or network involved.
pub async fn open_project(repo: &RepoRef) -> AppAction {
    last_opened_service()?.open_project(repo).await
}
