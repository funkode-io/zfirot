//! Disk-backed test for [`FileProjectStore::clear_account_data`]: it must clear
//! the last-opened, tracked-repos, and cached-recent-projects files while
//! leaving the theme and view-mode files untouched.

use std::time::{SystemTime, UNIX_EPOCH};

use application::ProjectStorePort;
use domain::{BoardViewMode, Project, RepoRef, ThemePreference};
use infrastructure::FileProjectStore;

#[tokio::test]
async fn clear_account_data_clears_account_state_but_keeps_ui_preferences() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after unix epoch")
        .as_nanos();
    let path = std::env::temp_dir()
        .join(format!("zfirot-project-store-{unique}"))
        .join("last_opened.json");
    let store = FileProjectStore::at(path.clone());

    let repo = RepoRef::new("funkode-io", "zfirot");
    store
        .remember_last_opened(&repo)
        .await
        .expect("seeding last-opened should succeed");
    store
        .track_repo(&repo)
        .await
        .expect("seeding a tracked repo should succeed");
    store
        .cache_projects(&[Project::new(repo.clone(), "2025-01-01T00:00:00Z")])
        .await
        .expect("seeding cached projects should succeed");
    store
        .remember_theme_preference(ThemePreference::Dark)
        .await
        .expect("seeding the theme preference should succeed");
    store
        .remember_view_mode(BoardViewMode::Graph)
        .await
        .expect("seeding the view mode should succeed");

    store
        .clear_account_data()
        .await
        .expect("clearing account data should succeed");

    assert_eq!(
        store.last_opened().await.expect("store should read"),
        None,
        "the last-opened project must be forgotten"
    );
    assert_eq!(
        store.tracked_repos().await.expect("store should read"),
        Vec::new(),
        "tracked repos must be cleared"
    );
    assert_eq!(
        store.cached_projects().await.expect("store should read"),
        None,
        "the cached recent-projects list must be cleared"
    );
    assert_eq!(
        store.theme_preference().await.expect("store should read"),
        Some(ThemePreference::Dark),
        "the theme preference must be preserved"
    );
    assert_eq!(
        store.view_mode().await.expect("store should read"),
        Some(BoardViewMode::Graph),
        "the board view mode must be preserved"
    );

    // Clearing an already-cleared store is safe (files already absent).
    store
        .clear_account_data()
        .await
        .expect("clearing an already-cleared store should be a no-op, not an error");

    let _ = std::fs::remove_dir_all(
        path.parent()
            .expect("the last-opened path should have a parent directory"),
    );
}
