use std::time::{SystemTime, UNIX_EPOCH};

use application::{BoardCachePort, BoardService};
use domain::RepoRef;
use infrastructure::{FakeBoardCache, FakeGitHubPort, FileBoardCache};

async fn snapshot_for(repo: &RepoRef) -> application::BoardSnapshot {
    BoardService::new(FakeGitHubPort)
        .load(repo)
        .await
        .expect("fixture load should succeed")
        .snapshot
}

#[tokio::test]
async fn fake_board_cache_round_trips_per_repo() {
    let cache = FakeBoardCache::empty();
    let repo_a = RepoRef::new("funkode-io", "zfirot");
    let repo_b = RepoRef::new("funkode-io", "replay");

    assert_eq!(
        cache
            .cached_board(&repo_a)
            .await
            .expect("cache read should succeed"),
        None,
        "fake cache starts cold"
    );

    let snapshot_a = snapshot_for(&repo_a).await;
    let snapshot_b = snapshot_for(&repo_b).await;
    cache
        .cache_board(&repo_a, &snapshot_a)
        .await
        .expect("cache write should succeed");
    cache
        .cache_board(&repo_b, &snapshot_b)
        .await
        .expect("cache write should succeed");

    assert_eq!(
        cache
            .cached_board(&repo_a)
            .await
            .expect("cache read should succeed"),
        Some(snapshot_a),
        "repo A snapshot should round-trip"
    );
    assert_eq!(
        cache
            .cached_board(&repo_b)
            .await
            .expect("cache read should succeed"),
        Some(snapshot_b),
        "repo B snapshot should round-trip"
    );
}

#[tokio::test]
async fn file_board_cache_round_trips_per_repo() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("zfirot-board-cache-{unique}"));
    let cache = FileBoardCache::at(root.clone());
    let repo_a = RepoRef::new("funkode-io", "zfirot");
    let repo_b = RepoRef::new("funkode-io", "replay");

    let snapshot_a = snapshot_for(&repo_a).await;
    let snapshot_b = snapshot_for(&repo_b).await;

    cache
        .cache_board(&repo_a, &snapshot_a)
        .await
        .expect("cache write should succeed");
    cache
        .cache_board(&repo_b, &snapshot_b)
        .await
        .expect("cache write should succeed");

    assert_eq!(
        cache
            .cached_board(&repo_a)
            .await
            .expect("cache read should succeed"),
        Some(snapshot_a),
        "repo A snapshot should round-trip"
    );
    assert_eq!(
        cache
            .cached_board(&repo_b)
            .await
            .expect("cache read should succeed"),
        Some(snapshot_b),
        "repo B snapshot should round-trip"
    );

    let _ = std::fs::remove_dir_all(root);
}
