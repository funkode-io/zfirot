//! The **hot** refresh: the open-PR **Sweep** that keeps every fact on the
//! critical path within the Freshness contract's 15 seconds (ADR 0008).
//!
//! Every case here asserts external behaviour — the board a given set of GitHub
//! facts produces, and whether the local cache was rewritten — never call
//! counts or query shapes. All offline, driven through the `GitHubPort` and
//! `BoardCachePort` fakes.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use application::{
    BoardCachePort, BoardCacheUsage, BoardOpen, BoardRefresh, BoardSnapshot, CachedBoardService,
    ClassifiedBoard, GitHubPort, LoadedBoard,
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use domain::{
    AppAction, AppError, AppResult, LinkedPrRef, PrStatus, Project, RawIssue, RawLinkedPr, RepoRef,
    SliceState,
};

#[derive(Default)]
struct CountingBoardCache {
    snapshots: Mutex<HashMap<String, BoardSnapshot>>,
    writes: AtomicUsize,
}

impl CountingBoardCache {
    fn writes(&self) -> usize {
        self.writes.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl BoardCachePort for CountingBoardCache {
    async fn cached_board(&self, repo: &RepoRef) -> AppResult<Option<BoardSnapshot>> {
        Ok(self
            .snapshots
            .lock()
            .expect("lock poisoned")
            .get(&repo.to_string())
            .cloned())
    }

    async fn cache_board(&self, repo: &RepoRef, snapshot: &BoardSnapshot) -> AppAction {
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

/// A GitHub whose answers are scripted per call: one issue set per full load,
/// one delta per issue-side refresh, and one open-PR **Sweep** per hot refresh.
/// Either side can be made to fail on its own, so the independence of the two
/// refresh paths is observable.
struct ScriptedGitHub {
    issues: Mutex<VecDeque<Vec<RawIssue>>>,
    deltas: Mutex<VecDeque<Vec<RawIssue>>>,
    sweeps: Mutex<VecDeque<Vec<RawLinkedPr>>>,
    issues_fail: bool,
    sweep_fails: bool,
}

impl ScriptedGitHub {
    fn new(issues: Vec<RawIssue>, sweeps: Vec<Vec<RawLinkedPr>>) -> Self {
        Self {
            issues: Mutex::new(VecDeque::from(vec![issues])),
            deltas: Mutex::new(VecDeque::new()),
            sweeps: Mutex::new(VecDeque::from(sweeps)),
            issues_fail: false,
            sweep_fails: false,
        }
    }

    /// The same GitHub, answering each issue-side delta from `deltas` in order.
    fn with_deltas(self, deltas: Vec<Vec<RawIssue>>) -> Self {
        Self {
            deltas: Mutex::new(VecDeque::from(deltas)),
            ..self
        }
    }

    /// The same GitHub, but every issue-side call fails (a broken structural
    /// path).
    fn with_failing_issues(self) -> Self {
        Self {
            issues_fail: true,
            ..self
        }
    }

    /// The same GitHub, but every open-PR sweep fails (a connectivity blip on
    /// the hot path).
    fn with_failing_sweep(self) -> Self {
        Self {
            sweep_fails: true,
            ..self
        }
    }
}

#[async_trait]
impl GitHubPort for ScriptedGitHub {
    async fn load_issues(&self, _repo: &RepoRef) -> AppResult<Vec<RawIssue>> {
        if self.issues_fail {
            return Err(AppError::unavailable("GitHub is temporarily unavailable"));
        }
        let mut issues = self.issues.lock().expect("lock poisoned");
        let next = issues
            .pop_front()
            .expect("issue sequence should have a value");
        // A full load is repeatable: the last scripted set keeps answering.
        if issues.is_empty() {
            issues.push_back(next.clone());
        }
        Ok(next)
    }

    async fn load_issues_since(
        &self,
        _repo: &RepoRef,
        _since: DateTime<Utc>,
    ) -> AppResult<Vec<RawIssue>> {
        if self.issues_fail {
            return Err(AppError::unavailable("GitHub is temporarily unavailable"));
        }
        Ok(self
            .deltas
            .lock()
            .expect("lock poisoned")
            .pop_front()
            .unwrap_or_default())
    }

    async fn sweep_open_prs(&self, _repo: &RepoRef) -> AppResult<Vec<RawLinkedPr>> {
        if self.sweep_fails {
            return Err(AppError::unavailable("GitHub is temporarily unavailable"));
        }
        let mut sweeps = self.sweeps.lock().expect("lock poisoned");
        let next = sweeps.pop_front().unwrap_or_default();
        if sweeps.is_empty() {
            sweeps.push_back(next.clone());
        }
        Ok(next)
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

fn repo() -> RepoRef {
    RepoRef::new("funkode-io", "zfirot")
}

fn slice_issue(number: u64) -> RawIssue {
    RawIssue {
        number,
        title: format!("Slice {number}"),
        url: format!("https://github.com/funkode-io/zfirot/issues/{number}"),
        body: None,
        labels: vec!["slice".to_string()],
        closed: false,
        native_parent: None,
        native_blockers: vec![],
        assignee: None,
        assignee_avatar_url: None,
        is_native_child_of_prd: false,
    }
}

/// An open PR closing `issue`, as the sweep reports it.
fn open_pr(number: u64, issue: u64) -> RawLinkedPr {
    RawLinkedPr {
        pr: LinkedPrRef {
            number,
            author: Some("colleague".to_string()),
            title: format!("PR {number}"),
            url: format!("https://github.com/funkode-io/zfirot/pull/{number}"),
            pr_status: PrStatus::AwaitingReview,
            conflicts: false,
            ci_failing: false,
            unresolved_comment_count: 0,
        },
        closes: vec![issue],
    }
}

fn slice(board: &ClassifiedBoard, number: u64) -> &domain::Slice {
    board
        .slices
        .iter()
        .find(|slice| slice.number == number)
        .unwrap_or_else(|| panic!("issue #{number} should be a Slice on the board"))
}

fn changed(refresh: BoardRefresh, why: &str) -> LoadedBoard {
    match refresh {
        BoardRefresh::Changed(loaded) => loaded,
        BoardRefresh::Unchanged(_) => panic!("{why}"),
    }
}

/// Open `repo` cold (seeding the cache) and return the seeded snapshot.
async fn seeded(
    service: &CachedBoardService<ScriptedGitHub, Arc<CountingBoardCache>>,
) -> BoardSnapshot {
    match service.open(&repo()).await.expect("cold open should seed") {
        BoardOpen::Cold(loaded) => loaded.snapshot,
        BoardOpen::Cached(_) => panic!("the first open of a project is cold"),
    }
}

#[tokio::test]
async fn a_pr_opened_by_someone_else_moves_a_ready_slice_to_wip() {
    let cache = Arc::new(CountingBoardCache::default());
    let service = CachedBoardService::new(
        ScriptedGitHub::new(vec![slice_issue(7)], vec![vec![], vec![open_pr(70, 7)]]),
        cache.clone(),
    );

    let snapshot = seeded(&service).await;
    assert_eq!(
        slice(&application::classify(&[slice_issue(7)], &[]), 7).state,
        SliceState::Ready,
        "a Slice nobody has started is Ready",
    );

    let hot = service
        .hot_refresh_cached(&repo(), &snapshot)
        .await
        .expect("hot refresh should succeed");

    let loaded = changed(
        hot,
        "a PR appearing on a Ready Slice must repaint it as WIP",
    );
    let slice = slice(&loaded.board, 7);
    assert_eq!(slice.state, SliceState::Wip, "an open PR is WIP");
    assert_eq!(
        slice
            .linked_prs
            .iter()
            .map(|pr| pr.number)
            .collect::<Vec<_>>(),
        vec![70],
        "the Slice shows the swept PR's badge",
    );
}

#[tokio::test]
async fn a_pr_closed_without_merging_releases_its_slice() {
    let cache = Arc::new(CountingBoardCache::default());
    let service = CachedBoardService::new(
        ScriptedGitHub::new(vec![slice_issue(8)], vec![vec![open_pr(80, 8)], vec![]]),
        cache.clone(),
    );

    let snapshot = seeded(&service).await;

    let hot = service
        .hot_refresh_cached(&repo(), &snapshot)
        .await
        .expect("hot refresh should succeed");

    let loaded = changed(hot, "a PR leaving the open set must repaint its Slice");
    let slice = slice(&loaded.board, 8);
    assert_eq!(
        slice.state,
        SliceState::Ready,
        "a PR closed without merging releases the Slice",
    );
    assert!(
        slice.linked_prs.is_empty(),
        "a PR absent from the sweep shows no badge",
    );
}

#[tokio::test]
async fn review_and_ci_facts_reach_the_board_on_the_next_hot_sweep() {
    struct Case {
        name: &'static str,
        swept: RawLinkedPr,
        expect: fn(&LinkedPrRef),
    }

    let cases = [
        Case {
            name: "a new unresolved review thread raises the Unresolved comments Decoration",
            swept: RawLinkedPr {
                pr: LinkedPrRef {
                    unresolved_comment_count: 2,
                    ..open_pr(90, 9).pr
                },
                closes: vec![9],
            },
            expect: |pr| assert_eq!(pr.unresolved_comment_count, 2),
        },
        Case {
            name: "checks settling red raise the CI failing Decoration",
            swept: RawLinkedPr {
                pr: LinkedPrRef {
                    ci_failing: true,
                    ..open_pr(90, 9).pr
                },
                closes: vec![9],
            },
            expect: |pr| assert!(pr.ci_failing),
        },
        Case {
            name: "a branch that stopped merging cleanly raises the Conflicts Decoration",
            swept: RawLinkedPr {
                pr: LinkedPrRef {
                    conflicts: true,
                    ..open_pr(90, 9).pr
                },
                closes: vec![9],
            },
            expect: |pr| assert!(pr.conflicts),
        },
        Case {
            name: "an approval moves the PR status to Approved",
            swept: RawLinkedPr {
                pr: LinkedPrRef {
                    pr_status: PrStatus::Approved,
                    ..open_pr(90, 9).pr
                },
                closes: vec![9],
            },
            expect: |pr| assert_eq!(pr.pr_status, PrStatus::Approved),
        },
    ];

    for case in cases {
        let cache = Arc::new(CountingBoardCache::default());
        let service = CachedBoardService::new(
            ScriptedGitHub::new(
                vec![slice_issue(9)],
                vec![vec![open_pr(90, 9)], vec![case.swept]],
            ),
            cache.clone(),
        );

        let snapshot = seeded(&service).await;
        let hot = service
            .hot_refresh_cached(&repo(), &snapshot)
            .await
            .expect("hot refresh should succeed");

        let loaded = changed(hot, case.name);
        let best = slice(&loaded.board, 9)
            .best_pr()
            .expect("the Slice's PR is still open")
            .clone();
        (case.expect)(&best);
    }
}

#[tokio::test]
async fn an_identical_open_pr_set_repaints_nothing_and_rewrites_no_cache() {
    let cache = Arc::new(CountingBoardCache::default());
    let service = CachedBoardService::new(
        ScriptedGitHub::new(vec![slice_issue(10)], vec![vec![open_pr(100, 10)]]),
        cache.clone(),
    );

    let snapshot = seeded(&service).await;
    let writes_after_seed = cache.writes();

    let hot = service
        .hot_refresh_cached(&repo(), &snapshot)
        .await
        .expect("hot refresh should succeed");

    match hot {
        BoardRefresh::Unchanged(unchanged) => assert_eq!(
            unchanged, snapshot,
            "an unchanged sweep keeps the snapshot exactly as it was",
        ),
        BoardRefresh::Changed(_) => panic!("an identical open-PR set must not repaint"),
    }
    assert_eq!(
        cache.writes(),
        writes_after_seed,
        "an unchanged hot refresh must not rewrite the cache",
    );
}

#[tokio::test]
async fn an_identical_open_pr_set_writes_nothing_even_when_the_issue_side_moved_on() {
    let cache = Arc::new(CountingBoardCache::default());
    let service = CachedBoardService::new(
        ScriptedGitHub::new(vec![slice_issue(18)], vec![vec![], vec![]])
            .with_deltas(vec![vec![slice_issue(19)]]),
        cache.clone(),
    );

    // A structural refresh lands while the hot loop is still holding the
    // snapshot it started its sweep from.
    let opened = seeded(&service).await;
    changed(
        service
            .refresh_cached(&repo(), &opened)
            .await
            .expect("structural refresh should succeed"),
        "a new issue changes the board",
    );
    let writes_before_sweep = cache.writes();

    let hot = service
        .hot_refresh_cached(&repo(), &opened)
        .await
        .expect("hot refresh should succeed");

    let cached = cache
        .cached_board(&repo())
        .await
        .expect("cache read should succeed")
        .expect("the cache holds the structural refresh's snapshot");
    match hot {
        BoardRefresh::Unchanged(unchanged) => assert_eq!(
            unchanged, cached,
            "an unchanged sweep still carries the freshest issue facts forward",
        ),
        BoardRefresh::Changed(_) => panic!(
            "only a changed open-PR set may repaint: issue facts the hot path merely carried \
             forward are not its news",
        ),
    }
    assert_eq!(
        cache.writes(),
        writes_before_sweep,
        "an unchanged hot refresh must not rewrite the cache, whatever the issue side did",
    );
}

#[tokio::test]
async fn a_hot_refresh_leaves_the_issue_side_watermark_where_it_was() {
    let cache = Arc::new(CountingBoardCache::default());
    let service = CachedBoardService::new(
        ScriptedGitHub::new(vec![slice_issue(11)], vec![vec![], vec![open_pr(110, 11)]]),
        cache.clone(),
    );

    let snapshot = seeded(&service).await;
    let hot = service
        .hot_refresh_cached(&repo(), &snapshot)
        .await
        .expect("hot refresh should succeed");

    let loaded = changed(hot, "a new PR changes the board");
    assert_eq!(
        loaded.snapshot.fetched_at, snapshot.fetched_at,
        "the hot path fetches no issues, so it must not advance the issue-side watermark",
    );
}

#[tokio::test]
async fn a_structural_refresh_completing_afterwards_keeps_the_hot_pr_facts() {
    let cache = Arc::new(CountingBoardCache::default());
    let service = CachedBoardService::new(
        ScriptedGitHub::new(vec![slice_issue(12)], vec![vec![], vec![open_pr(120, 12)]])
            .with_deltas(vec![vec![slice_issue(13)]]),
        cache.clone(),
    );

    // The board opens with no PRs; the hot loop then discovers one.
    let opened = seeded(&service).await;
    let hot = changed(
        service
            .hot_refresh_cached(&repo(), &opened)
            .await
            .expect("hot refresh should succeed"),
        "the discovered PR changes the board",
    );

    // A structural refresh that started before the hot one — it still holds the
    // pre-hot snapshot — completes now.
    let structural = changed(
        service
            .refresh_cached(&repo(), &opened)
            .await
            .expect("structural refresh should succeed"),
        "a new issue changes the board",
    );

    assert_eq!(
        slice(&structural.board, 12).state,
        SliceState::Wip,
        "a structural refresh must not revert the hot refresh's PR facts",
    );
    assert!(
        structural
            .board
            .slices
            .iter()
            .any(|slice| slice.number == 13),
        "the structural refresh still applies its own issue-side change",
    );

    let cached = cache
        .cached_board(&repo())
        .await
        .expect("cache read should succeed")
        .expect("the cache holds a snapshot");
    assert_eq!(
        cached, structural.snapshot,
        "the cache holds the merged board, not the reverted one",
    );
    let _ = hot;
}

#[tokio::test]
async fn a_hot_refresh_completing_afterwards_keeps_the_structural_issue_facts() {
    let cache = Arc::new(CountingBoardCache::default());
    let service = CachedBoardService::new(
        ScriptedGitHub::new(vec![slice_issue(16)], vec![vec![], vec![open_pr(160, 16)]])
            .with_deltas(vec![vec![slice_issue(17)]]),
        cache.clone(),
    );

    let opened = seeded(&service).await;
    changed(
        service
            .refresh_cached(&repo(), &opened)
            .await
            .expect("structural refresh should succeed"),
        "a new issue changes the board",
    );

    // A hot refresh that started before the structural one — it still holds the
    // pre-structural snapshot — completes now.
    let hot = changed(
        service
            .hot_refresh_cached(&repo(), &opened)
            .await
            .expect("hot refresh should succeed"),
        "the discovered PR changes the board",
    );

    assert!(
        hot.board.slices.iter().any(|slice| slice.number == 17),
        "a hot refresh must not revert the structural refresh's issue facts",
    );
    assert_eq!(
        slice(&hot.board, 16).state,
        SliceState::Wip,
        "the hot refresh still applies its own PR-side change",
    );
}

#[tokio::test]
async fn a_failed_hot_sweep_leaves_the_last_good_board_and_cache_untouched() {
    let cache = Arc::new(CountingBoardCache::default());
    let seeding = CachedBoardService::new(
        ScriptedGitHub::new(vec![slice_issue(14)], vec![vec![open_pr(140, 14)]]),
        cache.clone(),
    );
    let snapshot = seeded(&seeding).await;
    let writes_after_seed = cache.writes();

    let service = CachedBoardService::new(
        ScriptedGitHub::new(vec![slice_issue(14)], vec![]).with_failing_sweep(),
        cache.clone(),
    );
    let error = service
        .hot_refresh_cached(&repo(), &snapshot)
        .await
        .expect_err("a failing sweep must surface as an error");

    assert!(
        format!("{error:?}").contains("repo"),
        "the failure carries the repo in its cause chain: {error:?}",
    );
    assert_eq!(
        cache.writes(),
        writes_after_seed,
        "a failed hot refresh must not rewrite the cache",
    );
    assert_eq!(
        cache
            .cached_board(&repo())
            .await
            .expect("cache read should succeed")
            .expect("the cache still holds the last good snapshot"),
        snapshot,
        "a failed hot refresh leaves the last good board in place",
    );
}

#[tokio::test]
async fn a_broken_issue_side_does_not_stop_pr_facts_from_refreshing() {
    let cache = Arc::new(CountingBoardCache::default());
    let seeding = CachedBoardService::new(
        ScriptedGitHub::new(vec![slice_issue(15)], vec![vec![]]),
        cache.clone(),
    );
    let snapshot = seeded(&seeding).await;

    let service = CachedBoardService::new(
        ScriptedGitHub::new(vec![], vec![vec![open_pr(150, 15)]]).with_failing_issues(),
        cache.clone(),
    );

    service
        .refresh_cached(&repo(), &snapshot)
        .await
        .expect_err("the structural path is broken");

    let hot = changed(
        service
            .hot_refresh_cached(&repo(), &snapshot)
            .await
            .expect("the hot path is independent of the issue side"),
        "the discovered PR changes the board",
    );
    assert_eq!(
        slice(&hot.board, 15).state,
        SliceState::Wip,
        "PR facts still refresh while the issue side is failing",
    );
}
