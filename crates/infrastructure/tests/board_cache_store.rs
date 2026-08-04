use std::time::{SystemTime, UNIX_EPOCH};

use application::{BoardCachePort, BoardOpen, BoardService, CachedBoardService};
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

/// A board cached by a previous version of the app is in a format this version
/// cannot read (the snapshot now holds issues and Linked PRs as two collections,
/// ADR 0008). It must be discarded — the cache simply reads cold — so the board
/// cold-loads and reseeds instead of painting a half-understood snapshot.
#[tokio::test]
async fn file_board_cache_discards_a_snapshot_from_a_previous_app_version() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be after unix epoch")
        .as_nanos();
    let root = std::env::temp_dir().join(format!("zfirot-board-cache-stale-{unique}"));
    let repo = RepoRef::new("funkode-io", "zfirot");
    let cache = FileBoardCache::at(root.clone());

    // The format that shipped before the open-PR sweep: no version marker, no
    // Linked PR collection, PR facts nested inside each issue.
    let path = root.join(&repo.owner).join(format!("{}.json", repo.name));
    std::fs::create_dir_all(path.parent().expect("the path has a parent"))
        .expect("the cache directory should be creatable");
    std::fs::write(
        &path,
        r#"{
            "raw_issues": [ {
                "number": 1, "title": "A Slice", "url": "https://x/1", "body": null,
                "labels": ["slice"], "closed": false, "native_parent": null,
                "native_blockers": [], "assignee": null, "assignee_avatar_url": null,
                "linked_prs": [], "is_native_child_of_prd": false
            } ],
            "fetched_at": "2026-07-28T07:41:22Z"
        }"#,
    )
    .expect("the stale snapshot should be writable");

    let cached = cache
        .cached_board(&repo)
        .await
        .expect("reading a stale cache must not fail the board");

    assert_eq!(
        cached, None,
        "a snapshot from a previous app version must read as a cold cache"
    );

    let opened = CachedBoardService::new(FakeGitHubPort, cache)
        .open(&repo)
        .await
        .expect("open should fall back to a cold load");
    assert!(
        matches!(opened, BoardOpen::Cold(_)),
        "a discarded cache must cold-load and reseed"
    );

    let _ = std::fs::remove_dir_all(root);
}
