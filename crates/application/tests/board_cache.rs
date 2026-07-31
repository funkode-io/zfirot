use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use application::{
    BoardCachePort, BoardCacheUsage, BoardOpen, BoardRefresh, CachedBoardService, GitHubPort,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use domain::{
    AppAction, AppError, AppResult, LinkedPrRef, PrStatus, Project, RawIssue, RepoRef, SliceState,
};

#[derive(Default)]
struct CountingBoardCache {
    snapshots: Mutex<HashMap<String, application::BoardSnapshot>>,
    writes: AtomicUsize,
}

impl CountingBoardCache {
    fn writes(&self) -> usize {
        self.writes.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl BoardCachePort for CountingBoardCache {
    async fn cached_board(&self, repo: &RepoRef) -> AppResult<Option<application::BoardSnapshot>> {
        Ok(self
            .snapshots
            .lock()
            .expect("lock poisoned")
            .get(&repo.to_string())
            .cloned())
    }

    async fn cache_board(
        &self,
        repo: &RepoRef,
        snapshot: &application::BoardSnapshot,
    ) -> AppAction {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.snapshots
            .lock()
            .expect("lock poisoned")
            .insert(repo.to_string(), snapshot.clone());
        Ok(())
    }

    async fn cache_usage(&self) -> AppResult<BoardCacheUsage> {
        Ok(BoardCacheUsage::default())
    }

    async fn clear_board(&self, repo: &RepoRef) -> AppAction {
        self.snapshots
            .lock()
            .expect("lock poisoned")
            .remove(&repo.to_string());
        Ok(())
    }

    async fn clear_all(&self) -> AppAction {
        self.snapshots.lock().expect("lock poisoned").clear();
        Ok(())
    }
}

struct SequencePort {
    issues: Mutex<VecDeque<Vec<RawIssue>>>,
    deltas: Mutex<VecDeque<Vec<RawIssue>>>,
}

impl SequencePort {
    fn new(issues: Vec<Vec<RawIssue>>, deltas: Vec<Vec<RawIssue>>) -> Self {
        Self {
            issues: Mutex::new(VecDeque::from(issues)),
            deltas: Mutex::new(VecDeque::from(deltas)),
        }
    }
}

#[async_trait]
impl GitHubPort for SequencePort {
    async fn load_issues(&self, _repo: &RepoRef) -> AppResult<Vec<RawIssue>> {
        Ok(self
            .issues
            .lock()
            .expect("lock poisoned")
            .pop_front()
            .expect("issues sequence should have a value"))
    }

    async fn load_issues_since(
        &self,
        _repo: &RepoRef,
        _since: DateTime<Utc>,
    ) -> AppResult<Vec<RawIssue>> {
        Ok(self
            .deltas
            .lock()
            .expect("lock poisoned")
            .pop_front()
            .expect("delta sequence should have a value"))
    }

    async fn list_projects(&self) -> AppResult<Vec<Project>> {
        Ok(vec![])
    }

    async fn assign_self(&self, _repo: &RepoRef, _issue_number: u64) -> AppAction {
        Ok(())
    }

    async fn add_label(&self, _repo: &RepoRef, _issue_number: u64, _label: &str) -> AppAction {
        Ok(())
    }
}

/// A [`GitHubPort`] whose delta/full-load calls always fail (a connectivity
/// blip), for asserting that a failed fetch never advances `fetched_at` or
/// rewrites the cache.
struct FailingPort;

#[async_trait]
impl GitHubPort for FailingPort {
    async fn load_issues(&self, _repo: &RepoRef) -> AppResult<Vec<RawIssue>> {
        Err(AppError::unavailable("GitHub is temporarily unavailable"))
    }

    async fn load_issues_since(
        &self,
        _repo: &RepoRef,
        _since: DateTime<Utc>,
    ) -> AppResult<Vec<RawIssue>> {
        Err(AppError::unavailable("GitHub is temporarily unavailable"))
    }

    async fn list_projects(&self) -> AppResult<Vec<Project>> {
        Ok(vec![])
    }

    async fn assign_self(&self, _repo: &RepoRef, _issue_number: u64) -> AppAction {
        Ok(())
    }

    async fn add_label(&self, _repo: &RepoRef, _issue_number: u64, _label: &str) -> AppAction {
        Ok(())
    }
}

fn open_issue(number: u64, title: &str) -> RawIssue {
    RawIssue {
        number,
        title: title.to_string(),
        url: format!("https://github.com/funkode-io/zfirot/issues/{number}"),
        body: None,
        labels: vec!["slice".to_string()],
        closed: false,
        native_parent: None,
        native_blockers: vec![],
        assignee: None,
        assignee_avatar_url: None,
        linked_prs: vec![],
        is_native_child_of_prd: false,
    }
}

#[tokio::test]
async fn cold_cache_falls_back_to_load_and_seeds_cache() {
    let repo = RepoRef::new("funkode-io", "zfirot");
    let cache = Arc::new(CountingBoardCache::default());
    let service = CachedBoardService::new(
        SequencePort::new(vec![vec![open_issue(10, "Cold")]], vec![vec![]]),
        cache.clone(),
    );

    let opened = service.open(&repo).await.expect("open should succeed");

    match opened {
        BoardOpen::Cold(loaded) => {
            assert_eq!(
                loaded.board.slices.len(),
                1,
                "cold open should paint loaded board"
            );
        }
        BoardOpen::Cached(_) => panic!("cold cache must load from GitHub"),
    }

    assert_eq!(cache.writes(), 1, "cold open seeds cache once");
    assert!(
        cache
            .cached_board(&repo)
            .await
            .expect("cache read should succeed")
            .is_some(),
        "cold open should persist a snapshot"
    );
}

#[tokio::test]
async fn seeded_open_uses_cache_then_refreshes_and_rewrites_cache() {
    let repo = RepoRef::new("funkode-io", "zfirot");
    let cache = Arc::new(CountingBoardCache::default());
    let service = CachedBoardService::new(
        SequencePort::new(
            vec![vec![open_issue(20, "Seed")]],
            vec![vec![RawIssue {
                closed: true,
                ..open_issue(20, "Seed")
            }]],
        ),
        cache.clone(),
    );

    let seeded = service.open(&repo).await.expect("cold open should seed");
    let snapshot = match seeded {
        BoardOpen::Cold(loaded) => loaded.snapshot,
        BoardOpen::Cached(_) => panic!("first open is cold"),
    };

    let cached_open = service
        .open(&repo)
        .await
        .expect("cached open should succeed");
    let cached_snapshot = match cached_open {
        BoardOpen::Cached(loaded) => {
            assert_eq!(
                loaded.board.slices.len(),
                1,
                "cached open paints instantly from cache"
            );
            loaded.snapshot
        }
        BoardOpen::Cold(_) => panic!("seeded project must open from cache"),
    };

    assert_eq!(
        cache.writes(),
        1,
        "cached open should not rewrite cache before refresh"
    );

    let refresh = service
        .refresh_cached(&repo, &cached_snapshot)
        .await
        .expect("refresh should succeed");

    match refresh {
        BoardRefresh::Changed(loaded) => {
            assert!(
                loaded.board.slices.is_empty(),
                "delta should be applied on top of cache"
            );
        }
        BoardRefresh::Unchanged(_) => panic!("closing an issue in delta should change the board"),
    }

    assert!(cache.writes() >= 2, "successful refresh rewrites cache");

    let _ = snapshot;
}

#[tokio::test]
async fn cache_is_scoped_per_repo_for_switch_and_reopen() {
    let repo_a = RepoRef::new("funkode-io", "a");
    let repo_b = RepoRef::new("funkode-io", "b");
    let cache = Arc::new(CountingBoardCache::default());
    let service = CachedBoardService::new(
        SequencePort::new(
            vec![vec![open_issue(1, "A")], vec![open_issue(2, "B")]],
            vec![vec![]],
        ),
        cache.clone(),
    );

    let _ = service
        .open(&repo_a)
        .await
        .expect("first open seeds repo a");
    let _ = service
        .open(&repo_b)
        .await
        .expect("first open seeds repo b");

    let reopened_a = service
        .open(&repo_a)
        .await
        .expect("reopen should use cache");
    match reopened_a {
        BoardOpen::Cached(loaded) => {
            assert_eq!(
                loaded.board.slices[0].number, 1,
                "reopen should paint repo A cache"
            );
        }
        BoardOpen::Cold(_) => panic!("seeded repo a should reopen from cache"),
    }

    let switched_b = service
        .open(&repo_b)
        .await
        .expect("switch should use cache");
    match switched_b {
        BoardOpen::Cached(loaded) => {
            assert_eq!(
                loaded.board.slices[0].number, 2,
                "switch should paint repo B cache"
            );
        }
        BoardOpen::Cold(_) => panic!("seeded repo b should open from cache"),
    }
}

#[tokio::test]
async fn unchanged_refresh_advances_cached_fetched_at() {
    let repo = RepoRef::new("funkode-io", "zfirot");
    let cache = Arc::new(CountingBoardCache::default());
    let service = CachedBoardService::new(
        SequencePort::new(
            vec![vec![open_issue(30, "Stable")]],
            // Empty delta => the board facts are unchanged on refresh.
            vec![vec![]],
        ),
        cache.clone(),
    );

    let snapshot = match service.open(&repo).await.expect("cold open should seed") {
        BoardOpen::Cold(loaded) => loaded.snapshot,
        BoardOpen::Cached(_) => panic!("first open is cold"),
    };

    let refresh = service
        .refresh_cached(&repo, &snapshot)
        .await
        .expect("refresh should succeed");

    // Facts are unchanged, but the snapshot must carry an advanced `fetched_at`
    // so the next delta `since` window moves forward instead of growing.
    let advanced = match refresh {
        BoardRefresh::Unchanged(advanced) => advanced,
        BoardRefresh::Changed(_) => panic!("empty delta should leave the board unchanged"),
    };
    assert!(
        advanced.fetched_at > snapshot.fetched_at,
        "unchanged refresh must advance fetched_at",
    );

    let cached = cache
        .cached_board(&repo)
        .await
        .expect("cache read should succeed")
        .expect("cache should hold a snapshot");
    assert_eq!(
        cached.fetched_at, advanced.fetched_at,
        "unchanged refresh must persist the advanced snapshot to the cache",
    );
}

#[tokio::test]
async fn clearing_a_repo_cache_forces_a_cold_reopen_and_reseed() {
    let repo = RepoRef::new("funkode-io", "zfirot");
    let cache = Arc::new(CountingBoardCache::default());
    let service = CachedBoardService::new(
        SequencePort::new(
            vec![vec![open_issue(40, "Seed")], vec![open_issue(40, "Reload")]],
            vec![],
        ),
        cache.clone(),
    );

    let _ = service.open(&repo).await.expect("first open should seed");
    match service
        .open(&repo)
        .await
        .expect("warm open should use cache")
    {
        BoardOpen::Cached(_) => {}
        BoardOpen::Cold(_) => panic!("second open should use warm cache"),
    }

    cache
        .clear_board(&repo)
        .await
        .expect("clear board should succeed");

    match service.open(&repo).await.expect("reopen should succeed") {
        BoardOpen::Cold(loaded) => {
            assert_eq!(
                loaded.board.slices[0].title, "Reload",
                "after clear, open should fetch from source and reseed",
            );
        }
        BoardOpen::Cached(_) => panic!("after clear, reopen must be a cold load"),
    }
}

#[tokio::test]
async fn clearing_all_cache_forces_cold_reopen_and_reseed() {
    let repo_a = RepoRef::new("funkode-io", "zfirot");
    let repo_b = RepoRef::new("funkode-io", "replay");
    let cache = Arc::new(CountingBoardCache::default());
    let service = CachedBoardService::new(
        SequencePort::new(
            vec![
                vec![open_issue(50, "A-Seed")],
                vec![open_issue(60, "B-Seed")],
                vec![open_issue(50, "A-Reload")],
                vec![open_issue(60, "B-Reload")],
            ],
            vec![],
        ),
        cache.clone(),
    );

    let _ = service
        .open(&repo_a)
        .await
        .expect("first open should seed repo a");
    let _ = service
        .open(&repo_b)
        .await
        .expect("first open should seed repo b");
    cache.clear_all().await.expect("clear all should succeed");

    match service
        .open(&repo_a)
        .await
        .expect("reopen repo a should succeed")
    {
        BoardOpen::Cold(loaded) => assert_eq!(loaded.board.slices[0].title, "A-Reload"),
        BoardOpen::Cached(_) => panic!("after clear all, repo a reopen must be a cold load"),
    }
    match service
        .open(&repo_b)
        .await
        .expect("reopen repo b should succeed")
    {
        BoardOpen::Cold(loaded) => assert_eq!(loaded.board.slices[0].title, "B-Reload"),
        BoardOpen::Cached(_) => panic!("after clear all, repo b reopen must be a cold load"),
    }
}

#[tokio::test]
async fn reconcile_full_load_noops_when_cache_is_aligned() {
    let repo = RepoRef::new("funkode-io", "zfirot");
    let cache = Arc::new(CountingBoardCache::default());
    let service = CachedBoardService::new(
        SequencePort::new(
            vec![
                vec![open_issue(40, "Aligned")],
                vec![open_issue(40, "Aligned")],
            ],
            vec![],
        ),
        cache.clone(),
    );

    let snapshot = match service.open(&repo).await.expect("cold open should seed") {
        BoardOpen::Cold(loaded) => loaded.snapshot,
        BoardOpen::Cached(_) => panic!("first open is cold"),
    };
    assert_eq!(cache.writes(), 1, "seed should write exactly once");

    let reconcile = service
        .reconcile_cached(&repo, &snapshot)
        .await
        .expect("reconcile should succeed");

    match reconcile {
        BoardRefresh::Unchanged(returned) => {
            assert_eq!(
                returned, snapshot,
                "aligned reconcile should keep the cached snapshot"
            );
        }
        BoardRefresh::Changed(_) => panic!("aligned reconcile should be a no-op"),
    }

    assert_eq!(
        cache.writes(),
        1,
        "aligned reconcile should not rewrite the cache"
    );
}

#[tokio::test]
async fn reconcile_full_load_drops_ghost_issue_from_cache() {
    let repo = RepoRef::new("funkode-io", "zfirot");
    let cache = Arc::new(CountingBoardCache::default());
    let service = CachedBoardService::new(
        SequencePort::new(
            vec![
                vec![open_issue(50, "Real"), open_issue(51, "Ghost")],
                vec![open_issue(50, "Real")],
            ],
            vec![],
        ),
        cache.clone(),
    );

    let snapshot = match service.open(&repo).await.expect("cold open should seed") {
        BoardOpen::Cold(loaded) => loaded.snapshot,
        BoardOpen::Cached(_) => panic!("first open is cold"),
    };
    assert_eq!(cache.writes(), 1, "seed should write exactly once");

    let reconcile = service
        .reconcile_cached(&repo, &snapshot)
        .await
        .expect("reconcile should succeed");

    match reconcile {
        BoardRefresh::Changed(loaded) => {
            assert_eq!(
                loaded.board.slices.len(),
                1,
                "full reconcile should drop a cached ghost issue"
            );
            assert_eq!(loaded.board.slices[0].number, 50);
        }
        BoardRefresh::Unchanged(_) => panic!("ghost removal must be a changed reconcile"),
    }

    assert_eq!(
        cache.writes(),
        2,
        "changed reconcile should rewrite the cache with the full-load snapshot"
    );
}

/// Regression test for the bug behind a Slice showing without its linked PR:
/// GitHub does not bump an issue's own `updatedAt` when a PR is opened with a
/// closing reference to it, so a delta fetched via `load_issues_since`'s
/// `since` filter can miss the PR entirely (the issue simply never appears in
/// the delta payload). Only a full reconcile — which re-derives every issue's
/// `closedByPullRequestsReferences` from scratch — is guaranteed to catch it.
#[tokio::test]
async fn delta_refresh_misses_a_newly_linked_pr_but_reconcile_catches_it() {
    let repo = RepoRef::new("funkode-io", "zfirot");
    let cache = Arc::new(CountingBoardCache::default());
    let linked_pr = LinkedPrRef {
        number: 99,
        author: Some("carlos-verdes".to_string()),
        title: "Fix the thing".to_string(),
        url: "https://github.com/funkode-io/zfirot/pull/99".to_string(),
        pr_status: PrStatus::AwaitingReview,
        conflicts: false,
        ci_failing: false,
        unresolved_comment_count: 0,
    };
    let with_linked_pr = RawIssue {
        linked_prs: vec![linked_pr],
        ..open_issue(60, "Ready Slice")
    };

    let service = CachedBoardService::new(
        SequencePort::new(
            vec![
                // Cold open: no linked PR yet.
                vec![open_issue(60, "Ready Slice")],
                // Full reconcile: GitHub's current state, PR now linked.
                vec![with_linked_pr],
            ],
            // The background delta: empty, exactly as GitHub would return it
            // when the issue's own `updatedAt` never advanced.
            vec![vec![]],
        ),
        cache.clone(),
    );

    let snapshot = match service.open(&repo).await.expect("cold open should seed") {
        BoardOpen::Cold(loaded) => {
            assert_eq!(
                loaded.board.slices[0].state,
                SliceState::Ready,
                "no linked PR yet: the Slice starts Ready"
            );
            loaded.snapshot
        }
        BoardOpen::Cached(_) => panic!("first open is cold"),
    };

    let refresh = service
        .refresh_cached(&repo, &snapshot)
        .await
        .expect("delta refresh should succeed");
    let snapshot_after_delta = match refresh {
        BoardRefresh::Unchanged(returned) => returned,
        BoardRefresh::Changed(_) => {
            panic!("an empty delta must not fabricate a change")
        }
    };

    let reconcile = service
        .reconcile_cached(&repo, &snapshot_after_delta)
        .await
        .expect("reconcile should succeed");

    match reconcile {
        BoardRefresh::Changed(loaded) => {
            assert_eq!(
                loaded.board.slices[0].state,
                SliceState::Wip,
                "a full reconcile must pick up the linked PR the delta could not see"
            );
        }
        BoardRefresh::Unchanged(_) => {
            panic!("reconcile must catch a linked PR the delta window missed")
        }
    }
}

#[tokio::test]
async fn failed_delta_refresh_leaves_snapshot_and_cache_untouched() {
    let repo = RepoRef::new("funkode-io", "zfirot");
    let cache = Arc::new(CountingBoardCache::default());
    let seed_service = CachedBoardService::new(
        SequencePort::new(vec![vec![open_issue(50, "Seeded")]], vec![]),
        cache.clone(),
    );
    let snapshot = match seed_service
        .open(&repo)
        .await
        .expect("cold open should seed")
    {
        BoardOpen::Cold(loaded) => loaded.snapshot,
        BoardOpen::Cached(_) => panic!("first open is cold"),
    };
    assert_eq!(cache.writes(), 1, "seed should write exactly once");

    // A connectivity blip on the delta half of a refresh.
    let flaky_service = CachedBoardService::new(FailingPort, cache.clone());
    flaky_service
        .refresh_cached(&repo, &snapshot)
        .await
        .expect_err("a failing port must surface as an error, not a silent no-op");

    assert_eq!(
        cache.writes(),
        1,
        "a failed delta refresh must not rewrite the cache"
    );
    let cached = cache
        .cached_board(&repo)
        .await
        .expect("cache read should succeed")
        .expect("cache should still hold the seeded snapshot");
    assert_eq!(
        cached.fetched_at, snapshot.fetched_at,
        "a failed delta refresh must not advance the cached fetched_at"
    );
}

#[tokio::test]
async fn failed_reconcile_leaves_snapshot_and_cache_untouched() {
    let repo = RepoRef::new("funkode-io", "zfirot");
    let cache = Arc::new(CountingBoardCache::default());
    let seed_service = CachedBoardService::new(
        SequencePort::new(vec![vec![open_issue(51, "Seeded")]], vec![]),
        cache.clone(),
    );
    let snapshot = match seed_service
        .open(&repo)
        .await
        .expect("cold open should seed")
    {
        BoardOpen::Cold(loaded) => loaded.snapshot,
        BoardOpen::Cached(_) => panic!("first open is cold"),
    };
    assert_eq!(cache.writes(), 1, "seed should write exactly once");

    // A connectivity blip on the full-load reconcile.
    let flaky_service = CachedBoardService::new(FailingPort, cache.clone());
    flaky_service
        .reconcile_cached(&repo, &snapshot)
        .await
        .expect_err("a failing port must surface as an error, not a silent no-op");

    assert_eq!(
        cache.writes(),
        1,
        "a failed reconcile must not rewrite the cache"
    );
    let cached = cache
        .cached_board(&repo)
        .await
        .expect("cache read should succeed")
        .expect("cache should still hold the seeded snapshot");
    assert_eq!(
        cached.fetched_at, snapshot.fetched_at,
        "a failed reconcile must not advance the cached fetched_at"
    );
}
