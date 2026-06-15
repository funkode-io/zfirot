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
    /// Returns an `Unauthorized` error when `GITHUB_TOKEN` is absent — or present
    /// but empty/whitespace-only — so the UI can tell the user how to configure
    /// it instead of building a client that GitHub later rejects.
    pub fn from_env() -> AppResult<Self> {
        let token = resolve_token(std::env::var("GITHUB_TOKEN").ok())?;

        let client = GitHubClient::new(token)?;
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

/// Validate a raw `GITHUB_TOKEN` value, treating an absent variable and a
/// present-but-blank one the same: both yield the actionable `Unauthorized`
/// setup guidance rather than a token GitHub would later reject.
fn resolve_token(raw: Option<String>) -> AppResult<String> {
    match raw {
        Some(token) if !token.trim().is_empty() => Ok(token.trim().to_string()),
        _ => Err(AppError::unauthorized(
            "No GITHUB_TOKEN found.\n\n\
             1. Create a fine-grained Personal Access Token at\n   \
             https://github.com/settings/personal-access-tokens/new\n\
             2. Grant the repository read access to Issues, Pull requests, and Contents.\n\
             3. Set it as GITHUB_TOKEN in your .env file (copy .env.example).\n\
             4. Restart the app.",
        )
        .with_operation("AppState::from_env")),
    }
}

/// The outcome of wiring at startup, injected as Dioxus context so the root can
/// either load the board or explain why it cannot.
#[derive(Clone)]
pub enum Boot {
    /// Dependencies are wired; the board can load.
    Ready(AppState),
    /// Startup failed (e.g. no token); carry the original error so its kind is
    /// preserved end-to-end and the UI shows its message verbatim. `Arc` keeps
    /// `Boot` cloneable for Dioxus context without flattening the error.
    Failed(Arc<AppError>),
}

impl Boot {
    /// Wire from the environment, capturing any failure as the original error.
    pub fn from_env() -> Self {
        match AppState::from_env() {
            Ok(state) => Boot::Ready(state),
            Err(error) => Boot::Failed(Arc::new(error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::AppErrorKind;

    #[test]
    fn missing_token_yields_unauthorized_setup_guidance() {
        let error = resolve_token(None).unwrap_err();

        assert_eq!(error.kind(), AppErrorKind::Unauthorized);
        assert!(error.to_string().contains("No GITHUB_TOKEN found."));
    }

    #[test]
    fn empty_token_is_treated_as_missing() {
        let error = resolve_token(Some(String::new())).unwrap_err();

        assert_eq!(error.kind(), AppErrorKind::Unauthorized);
        assert!(error.to_string().contains("No GITHUB_TOKEN found."));
    }

    #[test]
    fn whitespace_only_token_is_treated_as_missing() {
        let error = resolve_token(Some("   \n\t ".to_string())).unwrap_err();

        assert_eq!(error.kind(), AppErrorKind::Unauthorized);
        assert!(error.to_string().contains("No GITHUB_TOKEN found."));
    }

    #[test]
    fn a_real_token_is_accepted_and_trimmed() {
        let token = resolve_token(Some("  ghp_example  ".to_string())).unwrap();

        assert_eq!(token, "ghp_example");
    }
}
