//! Home-screen use-cases against the fake ports: recent projects come back
//! most-recently-pushed first, and opening a project round-trips through the
//! project store as the last-opened one.

use application::ProjectsService;
use domain::RepoRef;
use infrastructure::{FakeGitHubPort, FakeProjectStore};

#[tokio::test]
async fn recent_projects_are_sorted_by_last_push_descending() {
    let service = ProjectsService::new(FakeGitHubPort, FakeProjectStore::empty());

    let projects = service
        .recent_projects()
        .await
        .expect("fake port should list projects");

    assert!(projects.len() >= 2, "expected several canned projects");

    // Ownership of ordering lives in the service: a descending sort on the
    // RFC-3339 `pushed_at` string puts the most recently active project first.
    let pushed: Vec<&str> = projects.iter().map(|p| p.pushed_at.as_str()).collect();
    let mut sorted = pushed.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(
        pushed, sorted,
        "projects must be most-recently-pushed first"
    );
}

#[tokio::test]
async fn last_opened_round_trips_through_the_store() {
    let service = ProjectsService::new(FakeGitHubPort, FakeProjectStore::empty());

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
