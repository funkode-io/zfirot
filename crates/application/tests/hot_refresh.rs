//! The **hot** refresh: the open-PR **Sweep** that keeps every fact on the
//! critical path within the Freshness contract's 15 seconds, and the **Cascade**
//! that a change to the open-PR set triggers (ADR 0008).
//!
//! Every case here asserts external behaviour — the board a given set of GitHub
//! facts produces, and whether the local cache was rewritten — never call
//! counts or query shapes. All offline, driven through the `GitHubPort` and
//! `BoardCachePort` fakes.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
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
    reads_fail: AtomicBool,
}

impl CountingBoardCache {
    /// From now on every read fails (a permissions or disk error), while writes
    /// still land, so a write made without consulting the cache is observable.
    fn break_reads(&self) {
        self.reads_fail.store(true, Ordering::SeqCst);
    }

    fn writes(&self) -> usize {
        self.writes.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl BoardCachePort for CountingBoardCache {
    async fn cached_board(&self, repo: &RepoRef) -> AppResult<Option<BoardSnapshot>> {
        if self.reads_fail.load(Ordering::SeqCst) {
            return Err(AppError::internal("Could not read the cached board."));
        }
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
/// A **Hydrate** reads the issues as they are on GitHub right now, which start as
/// the ones the board loads and move with [`ScriptedGitHub::update_issue`]; a
/// number it does not hold is one GitHub cannot resolve. Each side can be made to
/// fail on its own, so the independence of the refresh paths is observable.
struct ScriptedGitHub {
    issues: Mutex<VecDeque<Vec<RawIssue>>>,
    deltas: Mutex<VecDeque<Vec<RawIssue>>>,
    sweeps: Mutex<VecDeque<Vec<RawLinkedPr>>>,
    on_github: Mutex<HashMap<u64, RawIssue>>,
    issues_fail: bool,
    sweep_fails: bool,
    hydrate_fails: AtomicBool,
}

impl ScriptedGitHub {
    fn new(issues: Vec<RawIssue>, sweeps: Vec<Vec<RawLinkedPr>>) -> Self {
        Self {
            on_github: Mutex::new(issues.iter().map(|i| (i.number, i.clone())).collect()),
            issues: Mutex::new(VecDeque::from(vec![issues])),
            deltas: Mutex::new(VecDeque::new()),
            sweeps: Mutex::new(VecDeque::from(sweeps)),
            issues_fail: false,
            sweep_fails: false,
            hydrate_fails: AtomicBool::new(false),
        }
    }

    /// The same GitHub, answering each issue-side delta from `deltas` in order.
    fn with_deltas(self, deltas: Vec<Vec<RawIssue>>) -> Self {
        Self {
            deltas: Mutex::new(VecDeque::from(deltas)),
            ..self
        }
    }

    /// The same GitHub, with `issue` as it now stands on GitHub, for a later
    /// **Hydrate** to read.
    fn with_issue(self, issue: RawIssue) -> Self {
        self.update_issue(issue);
        self
    }

    /// `issue` changes on GitHub, between two refreshes.
    fn update_issue(&self, issue: RawIssue) {
        self.on_github
            .lock()
            .expect("lock poisoned")
            .insert(issue.number, issue);
    }

    /// `number` can no longer be resolved on GitHub (deleted or transferred).
    fn remove_issue(&self, number: u64) {
        self.on_github
            .lock()
            .expect("lock poisoned")
            .remove(&number);
    }

    /// Every **Hydrate** from now on fails (a connectivity blip), or works again.
    fn set_hydrate_failing(&self, fails: bool) {
        self.hydrate_fails.store(fails, Ordering::SeqCst);
    }

    /// The same GitHub, but every full load and delta fails (a broken structural
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

    async fn hydrate_issues(&self, _repo: &RepoRef, numbers: &[u64]) -> AppResult<Vec<RawIssue>> {
        if self.hydrate_fails.load(Ordering::SeqCst) {
            return Err(AppError::unavailable("GitHub is temporarily unavailable"));
        }
        let on_github = self.on_github.lock().expect("lock poisoned");
        Ok(numbers
            .iter()
            .filter_map(|number| on_github.get(number).cloned())
            .collect())
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

/// A Slice blocked by `blocker`, through a native dependency link.
fn slice_blocked_by(number: u64, blocker: u64) -> RawIssue {
    RawIssue {
        native_blockers: vec![blocker],
        ..slice_issue(number)
    }
}

fn closed(issue: RawIssue) -> RawIssue {
    RawIssue {
        closed: true,
        ..issue
    }
}

/// An open PR closing no issue, as the sweep reports it.
fn unlinked_pr(number: u64) -> RawLinkedPr {
    RawLinkedPr {
        closes: vec![],
        ..open_pr(number, 0)
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

/// A PR closing nothing has no Slice to move, so nothing but the Unlinked PR
/// list can change. The hot sweep owns that list as much as any other PR fact:
/// it must still repaint, and a PR that leaves the sweep must leave the list.
#[tokio::test]
async fn an_unlinked_pr_appears_and_leaves_with_the_hot_sweep() {
    let cache = Arc::new(CountingBoardCache::default());
    let service = CachedBoardService::new(
        ScriptedGitHub::new(
            vec![slice_issue(10)],
            vec![vec![], vec![unlinked_pr(99)], vec![]],
        ),
        cache.clone(),
    );

    let snapshot = seeded(&service).await;

    let appeared = changed(
        service
            .hot_refresh_cached(&repo(), &snapshot)
            .await
            .expect("hot refresh should succeed"),
        "a new Unlinked PR must repaint the board",
    );
    let unlinked: Vec<u64> = appeared
        .board
        .unlinked_prs
        .iter()
        .map(|pr| pr.number)
        .collect();
    assert_eq!(unlinked, vec![99]);
    assert!(
        slice(&appeared.board, 10).linked_prs.is_empty(),
        "a PR closing nothing lands on no Slice card"
    );

    let left = changed(
        service
            .hot_refresh_cached(&repo(), &appeared.snapshot)
            .await
            .expect("hot refresh should succeed"),
        "a merged Unlinked PR must repaint the board",
    );
    assert!(left.board.unlinked_prs.is_empty());
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

/// A real change adopts GitHub's order too, so the Unlinked PR list keeps the
/// most recently updated PR first.
#[tokio::test]
async fn a_changed_sweep_lists_unlinked_prs_in_sweep_order() {
    let service = CachedBoardService::new(
        ScriptedGitHub::new(
            vec![slice_issue(10)],
            vec![vec![], vec![unlinked_pr(99), unlinked_pr(101)]],
        ),
        Arc::new(CountingBoardCache::default()),
    );

    let snapshot = seeded(&service).await;
    let loaded = changed(
        service
            .hot_refresh_cached(&repo(), &snapshot)
            .await
            .expect("hot refresh should succeed"),
        "new Unlinked PRs must repaint the board",
    );

    let unlinked: Vec<u64> = loaded
        .board
        .unlinked_prs
        .iter()
        .map(|pr| pr.number)
        .collect();
    assert_eq!(unlinked, vec![99, 101]);
}

/// The sweep is ordered by `updatedAt`, so any bump on GitHub reshuffles it
/// without changing a single fact. The contract is about the open-PR **set**:
/// a reshuffle is not news.
#[tokio::test]
async fn a_reordered_open_pr_set_repaints_nothing_and_rewrites_no_cache() {
    let closing_both = |number| RawLinkedPr {
        closes: vec![10, 11],
        ..open_pr(number, 10)
    };
    let closing_both_reversed = |number| RawLinkedPr {
        closes: vec![11, 10],
        ..open_pr(number, 10)
    };
    let cache = Arc::new(CountingBoardCache::default());
    let service = CachedBoardService::new(
        ScriptedGitHub::new(
            vec![slice_issue(10), slice_issue(11)],
            vec![
                vec![closing_both(100), open_pr(101, 11)],
                vec![open_pr(101, 11), closing_both_reversed(100)],
            ],
        ),
        cache.clone(),
    );

    let snapshot = seeded(&service).await;
    let writes_after_seed = cache.writes();

    let hot = service
        .hot_refresh_cached(&repo(), &snapshot)
        .await
        .expect("hot refresh should succeed");

    assert!(
        matches!(hot, BoardRefresh::Unchanged(_)),
        "the same open PRs in another order must not repaint",
    );
    assert_eq!(
        cache.writes(),
        writes_after_seed,
        "the same open PRs in another order must not rewrite the cache",
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
        "the hot path sweeps no issues, so it must not advance the issue-side watermark",
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

/// Every path writes on top of the freshest facts in the cache. When the cache
/// cannot be read those facts are unknown, so a write would risk reverting the
/// other path's newer collection: each path must fail visibly and write nothing.
#[tokio::test]
async fn an_unreadable_cache_fails_every_merging_path_without_writing() {
    let cache = Arc::new(CountingBoardCache::default());
    let service = CachedBoardService::new(
        ScriptedGitHub::new(vec![slice_issue(10)], vec![vec![], vec![open_pr(100, 10)]]),
        cache.clone(),
    );

    let snapshot = seeded(&service).await;
    let writes_after_seed = cache.writes();
    cache.break_reads();

    service
        .hot_refresh_cached(&repo(), &snapshot)
        .await
        .expect_err("a hot refresh that cannot read the cache must fail");
    service
        .refresh_cached(&repo(), &snapshot)
        .await
        .expect_err("a structural refresh that cannot read the cache must fail");
    service
        .reconcile_cached(&repo(), &snapshot)
        .await
        .expect_err("a reconcile that cannot read the cache must fail");

    assert_eq!(
        cache.writes(),
        writes_after_seed,
        "no path may write what it could not merge with the cache",
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
        ScriptedGitHub::new(vec![slice_issue(15)], vec![vec![open_pr(150, 15)]])
            .with_failing_issues(),
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

/// The bug the whole spec exists for (#179): a PR merges, and the board goes from
/// "nothing to do" to the Slices it unblocked in one repaint.
#[tokio::test]
async fn a_merged_pr_closes_its_slice_and_readies_every_slice_it_blocked() {
    let service = CachedBoardService::new(
        ScriptedGitHub::new(
            vec![
                slice_issue(20),
                slice_blocked_by(21, 20),
                slice_blocked_by(22, 20),
                slice_issue(23),
            ],
            vec![vec![open_pr(200, 20)], vec![]],
        )
        .with_issue(closed(slice_issue(20))),
        Arc::new(CountingBoardCache::default()),
    );

    let snapshot = seeded(&service).await;
    let opened = application::classify(
        &[slice_issue(20), slice_blocked_by(21, 20)],
        &[open_pr(200, 20)],
    );
    assert_eq!(slice(&opened, 20).state, SliceState::Wip);
    assert_eq!(slice(&opened, 21).state, SliceState::Blocked);

    let loaded = changed(
        service
            .hot_refresh_cached(&repo(), &snapshot)
            .await
            .expect("hot refresh should succeed"),
        "a merged PR must repaint the board",
    );

    assert!(
        !loaded.board.slices.iter().any(|slice| slice.number == 20),
        "the merged PR's Slice is Done, so it leaves the board",
    );
    for number in [21, 22] {
        assert_eq!(
            slice(&loaded.board, number).state,
            SliceState::Ready,
            "#{number} was only blocked by the Slice that just closed",
        );
    }
    assert_eq!(slice(&loaded.board, 23).state, SliceState::Ready);
}

#[tokio::test]
async fn a_cascade_reaches_the_cache_so_a_reopened_board_stays_correct() {
    let cache = Arc::new(CountingBoardCache::default());
    let service = CachedBoardService::new(
        ScriptedGitHub::new(
            vec![slice_issue(20), slice_blocked_by(21, 20)],
            vec![vec![open_pr(200, 20)], vec![]],
        )
        .with_issue(closed(slice_issue(20))),
        cache.clone(),
    );

    let snapshot = seeded(&service).await;
    let loaded = changed(
        service
            .hot_refresh_cached(&repo(), &snapshot)
            .await
            .expect("hot refresh should succeed"),
        "a merged PR must repaint the board",
    );

    let reopened = match service.open(&repo()).await.expect("reopen should succeed") {
        BoardOpen::Cached(reopened) => reopened,
        BoardOpen::Cold(_) => panic!("the cascade's board should be cached"),
    };
    assert_eq!(reopened.board, loaded.board);
    assert_eq!(slice(&reopened.board, 21).state, SliceState::Ready);
}

/// A PR appearing hydrates the issue it closes: the Slice's own facts are
/// current on the card, not merely marked WIP.
#[tokio::test]
async fn a_pr_appearing_hydrates_the_facts_of_the_issue_it_closes() {
    let service = CachedBoardService::new(
        ScriptedGitHub::new(
            vec![slice_issue(24), slice_issue(25)],
            vec![vec![], vec![open_pr(240, 24)]],
        )
        .with_issue(RawIssue {
            title: "Retitled by whoever picked it up".to_string(),
            assignee: Some("colleague".to_string()),
            ..slice_blocked_by(24, 25)
        }),
        Arc::new(CountingBoardCache::default()),
    );

    let snapshot = seeded(&service).await;
    let loaded = changed(
        service
            .hot_refresh_cached(&repo(), &snapshot)
            .await
            .expect("hot refresh should succeed"),
        "a new PR must repaint the board",
    );

    let slice = slice(&loaded.board, 24);
    assert_eq!(
        slice.state,
        SliceState::Blocked,
        "the blocker it just hydrated outranks the new PR's WIP",
    );
    assert_eq!(
        slice
            .linked_prs
            .iter()
            .map(|pr| pr.number)
            .collect::<Vec<_>>(),
        vec![240],
    );
    assert_eq!(slice.title, "Retitled by whoever picked it up");
    assert_eq!(slice.assignee.as_deref(), Some("colleague"));
    assert_eq!(
        slice.blockers.iter().map(|b| b.number).collect::<Vec<_>>(),
        vec![25],
    );
}

/// The vanished PR's issues are only known from the previous set: the sweep that
/// reports the change no longer mentions them.
#[tokio::test]
async fn a_vanished_pr_hydrates_every_issue_it_closed_from_the_previous_set() {
    let closing_both = RawLinkedPr {
        closes: vec![26, 27],
        ..open_pr(260, 26)
    };
    let service = CachedBoardService::new(
        ScriptedGitHub::new(
            vec![slice_issue(26), slice_issue(27), slice_issue(28)],
            vec![vec![closing_both], vec![]],
        )
        .with_issue(closed(slice_issue(26)))
        .with_issue(closed(slice_issue(27))),
        Arc::new(CountingBoardCache::default()),
    );

    let snapshot = seeded(&service).await;
    let loaded = changed(
        service
            .hot_refresh_cached(&repo(), &snapshot)
            .await
            .expect("hot refresh should succeed"),
        "a merged PR must repaint the board",
    );

    let numbers: Vec<u64> = loaded.board.slices.iter().map(|s| s.number).collect();
    assert_eq!(numbers, vec![28], "both issues the PR closed are Done");
}

/// The diff is taken against what the cache holds, not what a slower caller
/// happens to be carrying, or a PR it never saw would never cascade.
#[tokio::test]
async fn a_cascade_finds_the_vanished_pr_in_the_freshest_set_not_the_callers() {
    let cache = Arc::new(CountingBoardCache::default());
    let service = CachedBoardService::new(
        ScriptedGitHub::new(
            vec![slice_issue(29)],
            vec![vec![], vec![open_pr(290, 29)], vec![]],
        ),
        cache.clone(),
    );

    // The caller still holds the board from before the PR appeared...
    let stale = seeded(&service).await;
    changed(
        service
            .hot_refresh_cached(&repo(), &stale)
            .await
            .expect("hot refresh should succeed"),
        "the PR appears",
    );

    // ...and by the next sweep the PR has merged, closing its issue.
    let after_merge = CachedBoardService::new(
        ScriptedGitHub::new(vec![], vec![vec![]]).with_issue(closed(slice_issue(29))),
        cache.clone(),
    );
    let loaded = changed(
        after_merge
            .hot_refresh_cached(&repo(), &stale)
            .await
            .expect("hot refresh should succeed"),
        "the PR vanished from the freshest set",
    );
    assert!(loaded.board.slices.is_empty(), "its Slice is Done");
}

/// A deleted or transferred issue is gone, not a failed refresh: the rest of the
/// batch still lands.
#[tokio::test]
async fn hydrating_an_unresolvable_issue_drops_it_and_refreshes_the_rest() {
    let closing_both = RawLinkedPr {
        closes: vec![30, 31],
        ..open_pr(300, 30)
    };
    let github = ScriptedGitHub::new(
        vec![slice_issue(30), slice_issue(31)],
        vec![vec![], vec![closing_both]],
    );
    github.update_issue(RawIssue {
        title: "Still here, and retitled".to_string(),
        ..slice_issue(31)
    });
    github.remove_issue(30);
    let service = CachedBoardService::new(github, Arc::new(CountingBoardCache::default()));

    let snapshot = seeded(&service).await;
    let loaded = changed(
        service
            .hot_refresh_cached(&repo(), &snapshot)
            .await
            .expect("an unresolvable number must not fail the refresh"),
        "a new PR must repaint the board",
    );

    assert!(
        !loaded.board.slices.iter().any(|slice| slice.number == 30),
        "the issue GitHub cannot resolve is treated as gone",
    );
    assert_eq!(slice(&loaded.board, 31).title, "Still here, and retitled");
}

/// Black-box proof of "no hydration at all": any hydrate would fail this GitHub.
#[tokio::test]
async fn an_unchanged_open_pr_set_triggers_no_hydration() {
    let github = ScriptedGitHub::new(vec![slice_issue(32)], vec![vec![open_pr(320, 32)]]);
    github.set_hydrate_failing(true);
    let service = CachedBoardService::new(github, Arc::new(CountingBoardCache::default()));

    let snapshot = seeded(&service).await;
    let hot = service
        .hot_refresh_cached(&repo(), &snapshot)
        .await
        .expect("an unchanged set hydrates nothing, so nothing can fail");

    assert!(matches!(hot, BoardRefresh::Unchanged(_)));
}

/// A PR's review and CI facts say nothing about its issue: only a change to which
/// PRs close which issues cascades, so a busy PR stays cheap.
#[tokio::test]
async fn a_changed_decoration_on_the_same_pr_triggers_no_hydration() {
    let reddened = RawLinkedPr {
        pr: LinkedPrRef {
            ci_failing: true,
            ..open_pr(330, 33).pr
        },
        closes: vec![33],
    };
    let github = ScriptedGitHub::new(
        vec![slice_issue(33)],
        vec![vec![open_pr(330, 33)], vec![reddened]],
    );
    github.set_hydrate_failing(true);
    let service = CachedBoardService::new(github, Arc::new(CountingBoardCache::default()));

    let snapshot = seeded(&service).await;
    let loaded = changed(
        service
            .hot_refresh_cached(&repo(), &snapshot)
            .await
            .expect("no cascade is due, so nothing can fail"),
        "a red CI must repaint the board",
    );
    assert!(slice(&loaded.board, 33).best_pr().expect("open").ci_failing);
}

/// A PR gaining or losing a closing reference moves the join itself, so both the
/// issue it now closes and the one it no longer closes are hydrated.
#[tokio::test]
async fn a_pr_retargeted_to_another_issue_hydrates_both_issues() {
    let github = ScriptedGitHub::new(
        vec![slice_issue(34), slice_issue(35)],
        vec![vec![open_pr(340, 34)], vec![open_pr(340, 35)]],
    )
    .with_issue(RawIssue {
        title: "Old target, retitled".to_string(),
        ..slice_issue(34)
    })
    .with_issue(RawIssue {
        title: "New target, retitled".to_string(),
        ..slice_issue(35)
    });
    let service = CachedBoardService::new(github, Arc::new(CountingBoardCache::default()));

    let snapshot = seeded(&service).await;
    let loaded = changed(
        service
            .hot_refresh_cached(&repo(), &snapshot)
            .await
            .expect("hot refresh should succeed"),
        "a retargeted PR must repaint the board",
    );

    assert_eq!(slice(&loaded.board, 34).title, "Old target, retitled");
    assert_eq!(slice(&loaded.board, 35).title, "New target, retitled");
    assert_eq!(slice(&loaded.board, 34).state, SliceState::Ready);
    assert_eq!(slice(&loaded.board, 35).state, SliceState::Wip);
}

/// Writing the swept PRs without their hydrate would make the next sweep look
/// unchanged, losing the cascade for good. A failed cascade therefore writes
/// nothing, and the very next tick finds the same change and tries again.
#[tokio::test]
async fn a_failed_cascade_writes_nothing_and_the_next_sweep_retries_it() {
    let cache = Arc::new(CountingBoardCache::default());
    let github = Arc::new(
        ScriptedGitHub::new(
            vec![slice_issue(36), slice_blocked_by(37, 36)],
            vec![vec![open_pr(360, 36)], vec![]],
        )
        .with_issue(closed(slice_issue(36))),
    );
    let service = CachedBoardService::new(github.clone(), cache.clone());

    let loaded = match service.open(&repo()).await.expect("cold open should seed") {
        BoardOpen::Cold(loaded) => loaded,
        BoardOpen::Cached(_) => panic!("the first open of a project is cold"),
    };
    let writes_after_seed = cache.writes();

    github.set_hydrate_failing(true);
    service
        .hot_refresh_cached(&repo(), &loaded.snapshot)
        .await
        .expect_err("a failed cascade must surface as an error");
    assert_eq!(
        cache.writes(),
        writes_after_seed,
        "a failed cascade must leave the last good board in the cache",
    );

    github.set_hydrate_failing(false);
    let retried = changed(
        service
            .hot_refresh_cached(&repo(), &loaded.snapshot)
            .await
            .expect("the retry should succeed"),
        "the retried cascade must repaint the board",
    );
    assert_eq!(slice(&retried.board, 37).state, SliceState::Ready);
}

/// A structural refresh that started before a cascade landed read the issue while
/// it was still open. Finishing later must not resurrect it, or the merged PR's
/// Slice would sit on the board again until the next delta.
#[tokio::test]
async fn an_issue_side_refresh_completing_after_a_cascade_keeps_its_issue_facts() {
    let cache = Arc::new(CountingBoardCache::default());
    let service = CachedBoardService::new(
        ScriptedGitHub::new(
            vec![slice_issue(38), slice_blocked_by(39, 38)],
            vec![vec![open_pr(380, 38)], vec![]],
        )
        .with_issue(closed(slice_issue(38)))
        .with_deltas(vec![vec![slice_issue(40)]]),
        cache.clone(),
    );

    let opened = seeded(&service).await;
    changed(
        service
            .hot_refresh_cached(&repo(), &opened)
            .await
            .expect("hot refresh should succeed"),
        "the merged PR's cascade changes the board",
    );

    // Both issue-side paths started from the pre-cascade snapshot.
    let delta = changed(
        service
            .refresh_cached(&repo(), &opened)
            .await
            .expect("structural refresh should succeed"),
        "a new issue changes the board",
    );
    assert!(!delta.board.slices.iter().any(|slice| slice.number == 38));
    assert_eq!(slice(&delta.board, 39).state, SliceState::Ready);
    assert!(
        delta.board.slices.iter().any(|slice| slice.number == 40),
        "the refresh still applies its own issue-side change",
    );

    let reconciled = changed(
        service
            .reconcile_cached(&repo(), &opened)
            .await
            .expect("reconcile should succeed"),
        "the reconcile's base predates the cascade",
    );
    assert!(
        !reconciled
            .board
            .slices
            .iter()
            .any(|slice| slice.number == 38),
        "a reload that read the issue before the cascade must not resurrect it",
    );
}
