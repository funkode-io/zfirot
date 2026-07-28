//! Use-case test for signing out, run against fakes — deterministic, offline,
//! and never touching the real keyring or filesystem.

use std::sync::Arc;

use application::{
    AccountService, BoardCachePort, BoardService, ProjectStorePort, SecureStorePort,
};
use domain::{BoardViewMode, GitHubToken, Project, RepoRef, ThemePreference};
use infrastructure::{FakeBoardCache, FakeGitHubPort, FakeProjectStore, FakeSecureStore};

/// A cached board snapshot for `repo`, built through the real load path (the
/// only public way to construct a [`application::BoardSnapshot`]).
async fn snapshot_for(repo: &RepoRef) -> application::BoardSnapshot {
    BoardService::new(FakeGitHubPort)
        .load(repo)
        .await
        .expect("fixture load should succeed")
        .snapshot
}

#[tokio::test]
async fn sign_out_clears_account_scoped_state_but_keeps_ui_preferences() {
    let secure_store = Arc::new(FakeSecureStore::with_token(
        GitHubToken::parse("github_pat_ABC123")
            .expect("a well-formed fine-grained PAT should parse"),
    ));
    let project_store = Arc::new(FakeProjectStore::empty());
    let board_cache = Arc::new(FakeBoardCache::empty());

    // Seed a token (above), tracked repos, last-opened, cached projects, a
    // cached board, a theme, and a view mode — the full account-scoped +
    // preference surface.
    let repo = RepoRef::new("funkode-io", "zfirot");
    project_store
        .remember_last_opened(&repo)
        .await
        .expect("seeding last-opened should succeed");
    project_store
        .track_repo(&repo)
        .await
        .expect("seeding a tracked repo should succeed");
    project_store
        .cache_projects(&[Project::new(repo.clone(), "2025-01-01T00:00:00Z")])
        .await
        .expect("seeding cached projects should succeed");
    project_store
        .remember_theme_preference(ThemePreference::Dark)
        .await
        .expect("seeding the theme preference should succeed");
    project_store
        .remember_view_mode(BoardViewMode::Graph)
        .await
        .expect("seeding the view mode should succeed");
    board_cache
        .cache_board(&repo, &snapshot_for(&repo).await)
        .await
        .expect("seeding a cached board should succeed");

    let service = AccountService::new(
        secure_store.clone(),
        project_store.clone(),
        board_cache.clone(),
    );

    service
        .sign_out()
        .await
        .expect("sign-out should succeed against healthy fakes");

    // Account-scoped state is gone.
    assert_eq!(
        secure_store.load_token().await.unwrap(),
        None,
        "the token must be removed"
    );
    assert_eq!(
        project_store.last_opened().await.unwrap(),
        None,
        "the last-opened project must be forgotten"
    );
    assert_eq!(
        project_store.tracked_repos().await.unwrap(),
        Vec::new(),
        "tracked repos must be cleared"
    );
    assert_eq!(
        project_store.cached_projects().await.unwrap(),
        None,
        "the cached recent-projects list must be cleared"
    );
    assert_eq!(
        board_cache.cached_board(&repo).await.unwrap(),
        None,
        "cached board snapshots must be cleared"
    );

    // UI preferences, which are not tied to the Viewer, survive.
    assert_eq!(
        project_store.theme_preference().await.unwrap(),
        Some(ThemePreference::Dark),
        "the theme preference must be preserved"
    );
    assert_eq!(
        project_store.view_mode().await.unwrap(),
        Some(BoardViewMode::Graph),
        "the board view mode must be preserved"
    );
}
