//! Composition root: builds the live GitHub adapter from a stored token.
//!
//! The token comes from the OS secure store (see
//! [`infrastructure::KeyringSecureStore`]), not the environment. The rest of the
//! app talks to an `Arc<dyn GitHubPort>`, so previews and tests can hand in a
//! fake instead of the real client.

use std::sync::Arc;

use application::{
    AuthService, BoardService, ClassifiedBoard, GitHubPort, LastOpenedService, ProjectStorePort,
    ProjectsRefresh, RecentProjectsService, SecureStorePort, TrackedProjectsService,
};
use domain::{AppAction, AppResult, GitHubToken, IssueClassification, Project, RepoRef};
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

/// The projects use-case with stale-while-revalidate caching, wired from a
/// stored Personal Access Token (the live fetch) and the on-device project store
/// (the cache). Listing repositories needs GitHub, so it needs a token.
fn recent_projects_service(
    token: &GitHubToken,
) -> AppResult<RecentProjectsService<Arc<dyn GitHubPort>, Arc<dyn ProjectStorePort>>> {
    let port: Arc<dyn GitHubPort> = Arc::new(GitHubClient::new(token.expose())?);
    Ok(RecentProjectsService::new(port, project_store()?))
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

    /// Load and classify the board for the wired project: confirmed Slices for
    /// the Kanban columns plus the "other open issues" bucket.
    pub async fn classify_board(&self) -> AppResult<ClassifiedBoard> {
        BoardService::new(self.port.clone())
            .classify_board(&self.repo)
            .await
    }

    /// Assign the authenticated user to a Ready Slice's issue, claiming it on
    /// GitHub. The caller re-polls the board on success.
    pub async fn assign_self(&self, issue_number: u64) -> AppAction {
        BoardService::new(self.port.clone())
            .assign_self(&self.repo, issue_number)
            .await
    }

    /// Confirm a suggested classification by adding its `prd`/`slice` label on
    /// GitHub. The caller re-polls the board on success so the issue is
    /// reclassified; the board is left unchanged on failure.
    pub async fn confirm_classification(
        &self,
        issue_number: u64,
        classification: &IssueClassification,
    ) -> AppAction {
        BoardService::new(self.port.clone())
            .confirm_classification(&self.repo, issue_number, classification)
            .await
    }
}

/// The recent-projects list cached on the last successful fetch, for an instant
/// first paint of the home screen, or `None` on a cold cache. A local store
/// read: no token or network involved.
pub async fn cached_projects() -> AppResult<Option<Vec<Project>>> {
    project_store()?.cached_projects().await
}

/// The repositories the user has summoned by name on the home screen, in
/// newest-added-first order. A local store read: no token or network involved.
pub async fn tracked_repos() -> AppResult<Vec<RepoRef>> {
    project_store()?.tracked_repos().await
}

/// Refresh the recent-projects list from GitHub, rewriting the local cache only
/// when it changed, and report the outcome. The caller holds the resolved token
/// (e.g. the cold-cache blocking fetch on the home screen).
pub async fn refresh_projects(token: &GitHubToken) -> AppResult<ProjectsRefresh> {
    recent_projects_service(token)?.refresh().await
}

/// Background stale-while-revalidate refresh: resolve the stored token, refresh
/// the recent-projects list from GitHub, and report whether it changed. Used
/// after the cached list has already painted, so a failure simply leaves the
/// cached list in place.
pub async fn refresh_recent_projects() -> AppResult<ProjectsRefresh> {
    let token = AuthService::new(secure_store()).require_token().await?;
    refresh_projects(&token).await
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

/// Open a project via the go-to (typed-repo) action: try to load the board to
/// verify access, and if successful, track the repo before remembering it as
/// last-opened. Returns the board on success; on failure (e.g. 404), returns the
/// error and does NOT track.
///
/// The orchestration lives in [`TrackedProjectsService`]; this only wires the
/// live adapter and store into it.
pub async fn open_and_track_project(
    token: &GitHubToken,
    repo: &RepoRef,
) -> AppResult<ClassifiedBoard> {
    let port: Arc<dyn GitHubPort> = Arc::new(GitHubClient::new(token.expose())?);
    TrackedProjectsService::new(port, project_store()?)
        .open_and_track(repo)
        .await
}

/// Assign the authenticated user to a Ready Slice's issue, claiming it on
/// GitHub. Reads the stored token, wires the live adapter scoped to `repo`, and
/// runs the assign-self use-case; the board is re-polled by the caller on
/// success and left unchanged on failure.
pub async fn assign_self(repo: &RepoRef, issue_number: u64) -> AppAction {
    let token = AuthService::new(secure_store()).require_token().await?;
    AppState::from_token(&token, repo.clone())?
        .assign_self(issue_number)
        .await
}

/// Confirm a suggested classification by adding the `prd`/`slice` label to the
/// issue on GitHub. Reads the stored token, wires the live adapter scoped to
/// `repo`, and runs the confirm use-case; the board is re-polled by the caller
/// on success (the issue then classifies tier-1) and left unchanged on failure.
pub async fn confirm_classification(
    repo: &RepoRef,
    issue_number: u64,
    classification: &IssueClassification,
) -> AppAction {
    let token = AuthService::new(secure_store()).require_token().await?;
    AppState::from_token(&token, repo.clone())?
        .confirm_classification(issue_number, classification)
        .await
}
