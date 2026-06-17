use application::ProjectStorePort;
use domain::RepoRef;
use infrastructure::FakeProjectStore;

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
    store
        .track_repo(&repo1)
        .await
        .expect("should track");
    assert_eq!(
        store.tracked_repos().await.expect("should read"),
        vec![repo1.clone()],
        "repo is tracked"
    );

    // Tracking again is idempotent (no duplicate)
    store
        .track_repo(&repo1)
        .await
        .expect("should track");
    assert_eq!(
        store.tracked_repos().await.expect("should read"),
        vec![repo1.clone()],
        "repo not duplicated"
    );

    // Track a second repo (prepended, newest-first)
    let repo2 = RepoRef::new("owner", "repo2");
    store
        .track_repo(&repo2)
        .await
        .expect("should track");
    assert_eq!(
        store.tracked_repos().await.expect("should read"),
        vec![repo2.clone(), repo1.clone()],
        "new repos are prepended (newest-first)"
    );

    // Untrack the first repo
    store
        .untrack_repo(&repo1)
        .await
        .expect("should untrack");
    assert_eq!(
        store.tracked_repos().await.expect("should read"),
        vec![repo2.clone()],
        "repo1 removed"
    );

    // Untracking a non-existent repo is safe
    let repo3 = RepoRef::new("owner", "repo3");
    store
        .untrack_repo(&repo3)
        .await
        .expect("should untrack");
    assert_eq!(
        store.tracked_repos().await.expect("should read"),
        vec![repo2.clone()],
        "untracking non-existent repo is safe"
    );
}

#[tokio::test]
async fn tracked_repos_constructor_seeds_state() {
    let repos = vec![
        RepoRef::new("a", "x"),
        RepoRef::new("b", "y"),
    ];
    let store = FakeProjectStore::with_tracked_repos(repos.clone());

    assert_eq!(
        store.tracked_repos().await.expect("should read"),
        repos,
        "constructor seeds the initial list"
    );
}
