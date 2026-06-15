//! Home-screen use-cases against the fakes: recent projects come back
//! most-recently-pushed first regardless of the adapter's order, and opening a
//! project round-trips through the project store as the last-opened one.

use application::{GitHubPort, LastOpenedService, ProjectsService};
use async_trait::async_trait;
use domain::{AppResult, Project, RepoRef, Slice};
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

    async fn list_projects(&self) -> AppResult<Vec<Project>> {
        // Out of order on purpose: oldest first, newest in the middle.
        Ok(vec![
            Project::new(RepoRef::new("acme", "old"), "2023-01-01T00:00:00Z"),
            Project::new(RepoRef::new("acme", "new"), "2025-01-01T00:00:00Z"),
            Project::new(RepoRef::new("acme", "mid"), "2024-01-01T00:00:00Z"),
        ])
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
