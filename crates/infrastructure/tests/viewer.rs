//! Use-case test for the signed-in Viewer, run against fake `GitHubPort`s —
//! deterministic, offline, and never touching live GitHub.

use application::{AccountService, GitHubPort};
use async_trait::async_trait;
use domain::{
    AppAction, AppError, AppErrorKind, AppResult, Project, RawIssue, RawLinkedPr, RepoRef, Viewer,
};
use infrastructure::{FakeBoardCache, FakeProjectStore, FakeSecureStore};

/// A fake that returns a canned Viewer, standing in for a successful GitHub
/// identity fetch.
struct StubGitHubPort;

#[async_trait]
impl GitHubPort for StubGitHubPort {
    async fn load_issues(&self, _repo: &RepoRef) -> AppResult<Vec<RawIssue>> {
        Ok(vec![])
    }

    async fn sweep_open_prs(&self, _repo: &RepoRef) -> AppResult<Vec<RawLinkedPr>> {
        Ok(Vec::new())
    }

    async fn list_projects(&self) -> AppResult<Vec<Project>> {
        Ok(vec![])
    }

    async fn assign_self(&self, _repo: &RepoRef, _issue_number: u64) -> AppAction {
        Ok(())
    }

    async fn add_label(&self, _repo: &RepoRef, _issue_number: u64, _label: &str) -> AppAction {
        Ok(())
    }

    async fn viewer(&self) -> AppResult<Viewer> {
        Ok(Viewer {
            login: "carlos-verdes".to_string(),
            name: Some("Carlos Verdes".to_string()),
            avatar_url: "https://avatars.githubusercontent.com/u/9919?v=4".to_string(),
        })
    }
}

/// A fake whose `viewer()` fails, standing in for a token that cannot read
/// profile data (or a transient failure) — the account menu must fall back to
/// a generic icon rather than blocking.
struct FailingViewerPort;

#[async_trait]
impl GitHubPort for FailingViewerPort {
    async fn load_issues(&self, _repo: &RepoRef) -> AppResult<Vec<RawIssue>> {
        Ok(vec![])
    }

    async fn sweep_open_prs(&self, _repo: &RepoRef) -> AppResult<Vec<RawLinkedPr>> {
        Ok(Vec::new())
    }

    async fn list_projects(&self) -> AppResult<Vec<Project>> {
        Ok(vec![])
    }

    async fn assign_self(&self, _repo: &RepoRef, _issue_number: u64) -> AppAction {
        Ok(())
    }

    async fn add_label(&self, _repo: &RepoRef, _issue_number: u64, _label: &str) -> AppAction {
        Ok(())
    }

    async fn viewer(&self) -> AppResult<Viewer> {
        Err(AppError::unavailable("Could not reach GitHub"))
    }
}

#[tokio::test]
async fn viewer_returns_the_ports_signed_in_identity() {
    let service = AccountService::new(
        FakeSecureStore::empty(),
        FakeProjectStore::empty(),
        FakeBoardCache::empty(),
    );

    let viewer = service
        .viewer(&StubGitHubPort)
        .await
        .expect("the port should return a Viewer");

    assert_eq!(viewer.login, "carlos-verdes");
    assert_eq!(viewer.name.as_deref(), Some("Carlos Verdes"));
    assert_eq!(
        viewer.avatar_url,
        "https://avatars.githubusercontent.com/u/9919?v=4"
    );
}

#[tokio::test]
async fn viewer_surfaces_a_port_failure_with_operation_context() {
    let service = AccountService::new(
        FakeSecureStore::empty(),
        FakeProjectStore::empty(),
        FakeBoardCache::empty(),
    );

    let error = service
        .viewer(&FailingViewerPort)
        .await
        .expect_err("a failing port should surface as an error, not a panic");

    assert_eq!(error.kind(), AppErrorKind::Unavailable);
    assert!(
        format!("{error:?}").contains("AccountService::viewer"),
        "the use-case should annotate the operation for diagnostics"
    );
}
