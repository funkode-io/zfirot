//! Composition root: where the live adapter is wired and injected into the UI.
//!
//! This is the only place that reads the environment. [`AppState::from_env`]
//! resolves the `GITHUB_TOKEN` and builds the real [`GitHubClient`]; the rest of
//! the app talks to an `Arc<dyn GitHubPort>`, so previews and tests can hand in
//! a fake instead.

use std::sync::Arc;

use application::{BoardService, GitHubPort};
use domain::{AppError, AppResult, RepoRef, Slice};
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
    /// Wire the live GitHub adapter from the environment.
    ///
    /// Returns an `Unauthorized` error when `GITHUB_TOKEN` is absent so the UI
    /// can tell the user how to configure it.
    pub fn from_env() -> AppResult<Self> {
        let token = std::env::var("GITHUB_TOKEN").map_err(|_| {
            AppError::unauthorized(
                "No GITHUB_TOKEN found.\n\n\
                 1. Create a fine-grained Personal Access Token at\n   \
                 https://github.com/settings/personal-access-tokens/new\n\
                 2. Grant the repository read access to Issues, Pull requests, and Contents.\n\
                 3. Set it as GITHUB_TOKEN in your .env file (copy .env.example).\n\
                 4. Restart the app.",
            )
            .with_operation("AppState::from_env")
        })?;

        let client = GitHubClient::new(token, reqwest::Client::new());
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

/// The outcome of wiring at startup, injected as Dioxus context so the root can
/// either load the board or explain why it cannot.
#[derive(Clone)]
pub enum Boot {
    /// Dependencies are wired; the board can load.
    Ready(AppState),
    /// Startup failed (e.g. no token); show this user-safe message.
    Failed(String),
}

impl Boot {
    /// Wire from the environment, capturing any failure as a displayable message.
    pub fn from_env() -> Self {
        match AppState::from_env() {
            Ok(state) => Boot::Ready(state),
            Err(error) => Boot::Failed(error.to_string()),
        }
    }
}
