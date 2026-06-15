//! Home-screen use-cases against the fakes: recent projects come back
//! most-recently-pushed first regardless of the adapter's order, and opening a
//! project round-trips through the project store as the last-opened one.

use application::{
    GitHubPort, LastOpenedService, ProjectStorePort, ProjectsRefresh, ProjectsService,
    RecentProjectsService,
};
use async_trait::async_trait;
use domain::{AppAction, AppResult, Project, RawIssue, RepoRef, Slice};
use infrastructure::FakeProjectStore;

/// A GitHub port that returns projects in a deliberately *unsorted* order, so
/// the test fails if `ProjectsService` ever stops owning the recency sort
/// (a fake whose list happens to be pre-sorted could not catch that).
struct UnsortedGitHubPort;

#[async_trait]
impl GitHubPort for UnsortedGitHubPort {
    async fn load_board(&self, _repo: &RepoRef) -> AppResult<Vec<Slice>> {
        Ok(Vec::new())
    }

    async fn load_issues(&self, _repo: &RepoRef) -> AppResult<Vec<RawIssue>> {
        Ok(Vec::new())
    }

    async fn list_projects(&self) -> AppResult<Vec<Project>> {
        // Out of order on purpose: oldest first, newest in the middle.
        Ok(vec![
            Project::new(RepoRef::new("acme", "old"), "2023-01-01T00:00:00Z"),
            Project::new(RepoRef::new("acme", "new"), "2025-01-01T00:00:00Z"),
            Project::new(RepoRef::new("acme", "mid"), "2024-01-01T00:00:00Z"),
        ])
    }

    async fn assign_self(&self, _repo: &RepoRef, _issue_number: u64) -> AppAction {
        Ok(())
    }
}

#[tokio::test]
async fn recent_projects_are_sorted_by_last_push_descending() {
    let service = ProjectsService::new(UnsortedGitHubPort);

    let projects = service
        .recent_projects()
        .await
        .expect("port should list projects");

    // The service must impose the order, not echo the adapter's: most recently
    // pushed first, even though the port returned them out of order.
    let order: Vec<&str> = projects.iter().map(|p| p.repo.name.as_str()).collect();
    assert_eq!(
        order,
        ["new", "mid", "old"],
        "projects must be most-recently-pushed first"
    );
}

#[tokio::test]
async fn last_opened_round_trips_through_the_store() {
    let service = LastOpenedService::new(FakeProjectStore::empty());

    assert_eq!(
        service.last_opened().await.expect("store should read"),
        None,
        "nothing opened yet"
    );

    let repo = RepoRef::new("funkode-io", "zfirot");
    service
        .open_project(&repo)
        .await
        .expect("opening should persist the choice");

    assert_eq!(
        service.last_opened().await.expect("store should read"),
        Some(repo),
        "the opened project is remembered for the next launch"
    );
}

/// The live list the [`UnsortedGitHubPort`] yields once `RecentProjectsService`
/// has applied the recency sort: most-recently-pushed first.
fn sorted_live() -> Vec<Project> {
    vec![
        Project::new(RepoRef::new("acme", "new"), "2025-01-01T00:00:00Z"),
        Project::new(RepoRef::new("acme", "mid"), "2024-01-01T00:00:00Z"),
        Project::new(RepoRef::new("acme", "old"), "2023-01-01T00:00:00Z"),
    ]
}

#[tokio::test]
async fn cached_projects_round_trip_through_the_store() {
    let store = FakeProjectStore::empty();

    assert_eq!(
        store.cached_projects().await.expect("store should read"),
        None,
        "the cache starts cold"
    );

    store
        .cache_projects(&sorted_live())
        .await
        .expect("caching should persist the list");

    assert_eq!(
        store.cached_projects().await.expect("store should read"),
        Some(sorted_live()),
        "the cached list reads back unchanged"
    );
}

#[tokio::test]
async fn refresh_seeds_a_cold_cache_and_reports_changed() {
    let service = RecentProjectsService::new(UnsortedGitHubPort, FakeProjectStore::empty());

    assert_eq!(
        service.cached().await.expect("cache should read"),
        None,
        "nothing cached on a cold start"
    );

    assert_eq!(
        service.refresh().await.expect("refresh should fetch"),
        ProjectsRefresh::Changed(sorted_live()),
        "a cold cache always reports the live list as a change"
    );

    assert_eq!(
        service.cached().await.expect("cache should read"),
        Some(sorted_live()),
        "the live list is now cached for an instant next launch"
    );
}

#[tokio::test]
async fn refresh_reports_unchanged_when_the_cache_already_matches() {
    let store = FakeProjectStore::empty();
    store
        .cache_projects(&sorted_live())
        .await
        .expect("seeding the cache should persist");
    let service = RecentProjectsService::new(UnsortedGitHubPort, store);

    assert_eq!(
        service.refresh().await.expect("refresh should fetch"),
        ProjectsRefresh::Unchanged,
        "a live list equal to the cache is a no-op, so the UI does not flicker"
    );
}

#[tokio::test]
async fn refresh_rewrites_a_stale_cache_and_reports_changed() {
    let store = FakeProjectStore::empty();
    let stale = vec![Project::new(
        RepoRef::new("acme", "gone"),
        "2020-01-01T00:00:00Z",
    )];
    store
        .cache_projects(&stale)
        .await
        .expect("seeding the cache should persist");
    let service = RecentProjectsService::new(UnsortedGitHubPort, store);

    assert_eq!(
        service.refresh().await.expect("refresh should fetch"),
        ProjectsRefresh::Changed(sorted_live()),
        "a cache that differs from the live list is reported as a change"
    );

    assert_eq!(
        service.cached().await.expect("cache should read"),
        Some(sorted_live()),
        "the stale cache was overwritten with the fresh list"
    );
}
