use application::{GitHubPort, ProjectStorePort, TrackedProjectsService};
use async_trait::async_trait;
use domain::{AppAction, AppError, AppResult, Project, RawIssue, RepoRef, Slice};
use infrastructure::FakeProjectStore;
use std::sync::Arc;

#[tokio::test]
async fn tracked_repos_persist_and_are_idempotent() {
    let store = FakeProjectStore::empty();

    // Empty at first
    assert_eq!(
        store.tracked_repos().await.expect("should read"),
        Vec::new(),
        "no tracked repos initially"
    );

    // Track a repo
    let repo1 = RepoRef::new("owner", "repo1");
    store.track_repo(&repo1).await.expect("should track");
    assert_eq!(
        store.tracked_repos().await.expect("should read"),
        vec![repo1.clone()],
        "repo is tracked"
    );

    // Tracking again is idempotent (no duplicate)
    store.track_repo(&repo1).await.expect("should track");
    assert_eq!(
        store.tracked_repos().await.expect("should read"),
        vec![repo1.clone()],
        "repo not duplicated"
    );

    // Track a second repo (prepended, newest-first)
    let repo2 = RepoRef::new("owner", "repo2");
    store.track_repo(&repo2).await.expect("should track");
    assert_eq!(
        store.tracked_repos().await.expect("should read"),
        vec![repo2.clone(), repo1.clone()],
        "new repos are prepended (newest-first)"
    );

    // Untrack the first repo
    store.untrack_repo(&repo1).await.expect("should untrack");
    assert_eq!(
        store.tracked_repos().await.expect("should read"),
        vec![repo2.clone()],
        "repo1 removed"
    );

    // Untracking a non-existent repo is safe
    let repo3 = RepoRef::new("owner", "repo3");
    store.untrack_repo(&repo3).await.expect("should untrack");
    assert_eq!(
        store.tracked_repos().await.expect("should read"),
        vec![repo2.clone()],
        "untracking non-existent repo is safe"
    );
}

#[tokio::test]
async fn tracked_repos_constructor_seeds_state() {
    let repos = vec![RepoRef::new("a", "x"), RepoRef::new("b", "y")];
    let store = FakeProjectStore::with_tracked_repos(repos.clone());

    assert_eq!(
        store.tracked_repos().await.expect("should read"),
        repos,
        "constructor seeds the initial list"
    );
}

/// A GitHub port whose board load either succeeds (empty board) or fails like a
/// missing/forbidden repo, so the use-case test can drive both open paths.
struct StubGitHubPort {
    accessible: bool,
}

#[async_trait]
impl GitHubPort for StubGitHubPort {
    async fn load_board(&self, _repo: &RepoRef) -> AppResult<Vec<Slice>> {
        Ok(Vec::new())
    }

    async fn load_issues(&self, _repo: &RepoRef) -> AppResult<Vec<RawIssue>> {
        if self.accessible {
            Ok(Vec::new())
        } else {
            Err(AppError::not_found("Repository not found."))
        }
    }

    async fn list_projects(&self) -> AppResult<Vec<Project>> {
        Ok(Vec::new())
    }

    async fn assign_self(&self, _repo: &RepoRef, _issue_number: u64) -> AppAction {
        Ok(())
    }

    async fn add_label(&self, _repo: &RepoRef, _issue_number: u64, _label: &str) -> AppAction {
        Ok(())
    }
}

#[tokio::test]
async fn open_and_track_tracks_on_successful_open() {
    let store = Arc::new(FakeProjectStore::empty());
    let repo = RepoRef::new("owner", "repo");
    let service = TrackedProjectsService::new(StubGitHubPort { accessible: true }, store.clone());

    service
        .open_and_track(&repo)
        .await
        .expect("an accessible repo opens");

    assert_eq!(
        store.tracked_repos().await.expect("should read"),
        vec![repo.clone()],
        "a successful open tracks the repo",
    );
    assert_eq!(
        store.last_opened().await.expect("should read"),
        Some(repo),
        "a successful open is remembered as last-opened",
    );
}

#[tokio::test]
async fn open_and_track_does_not_track_a_failed_open() {
    let store = Arc::new(FakeProjectStore::empty());
    let repo = RepoRef::new("owner", "missing");
    let service = TrackedProjectsService::new(StubGitHubPort { accessible: false }, store.clone());

    service
        .open_and_track(&repo)
        .await
        .expect_err("a missing repo does not open");

    assert_eq!(
        store.tracked_repos().await.expect("should read"),
        Vec::new(),
        "a failed open tracks nothing",
    );
    assert_eq!(
        store.last_opened().await.expect("should read"),
        None,
        "a failed open is not remembered as last-opened",
    );
}
