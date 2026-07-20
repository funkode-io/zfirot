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
async fn fake_board_cache_reports_usage_and_supports_clear_one_and_all() {
    let cache = FakeBoardCache::empty();
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

    let usage = cache
        .cache_usage()
        .await
        .expect("cache usage should succeed");
    assert_eq!(usage.projects.len(), 2, "both repos should be reported");
    assert_eq!(
        usage.total_bytes,
        usage
            .projects
            .iter()
            .map(|project| project.bytes)
            .sum::<u64>(),
        "total bytes should equal the per-project sum",
    );
    assert!(
        usage
            .projects
            .iter()
            .any(|project| project.repo == repo_a && project.bytes > 0),
        "repo A usage should be reported with non-zero size",
    );
    assert!(
        usage
            .projects
            .iter()
            .any(|project| project.repo == repo_b && project.bytes > 0),
        "repo B usage should be reported with non-zero size",
    );

    cache
        .clear_board(&repo_a)
        .await
        .expect("clear one should succeed");
    assert!(
        cache
            .cached_board(&repo_a)
            .await
            .expect("cache read should succeed")
            .is_none(),
        "clearing repo A should remove only repo A",
    );
    assert!(
        cache
            .cached_board(&repo_b)
            .await
            .expect("cache read should succeed")
            .is_some(),
        "clearing repo A should keep repo B",
    );

    cache.clear_all().await.expect("clear all should succeed");
    assert!(
        cache
            .cached_board(&repo_b)
            .await
            .expect("cache read should succeed")
            .is_none(),
        "clear all should empty the cache",
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
